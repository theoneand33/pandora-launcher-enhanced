use std::{
    borrow::Cow,
    io::{BufRead, BufReader, PipeReader},
    sync::Arc,
};

use bridge::{
    game_output::GameOutputLogLevel,
    handle::FrontendHandle,
    message::{GameOutputMsg, MessageToFrontend},
};
use chrono::Utc;
use regex::Regex;
use std::sync::LazyLock;
use thiserror::Error;

// Maximum size for accumulated buffers to prevent unbounded growth
const MAX_ACCUMULATOR_SIZE: usize = 10 * 1024 * 1024; // 10 MB

static REPLACEMENTS: LazyLock<[(Regex, &'static str); 7]> = LazyLock::new(|| {
    [
        // Access token replacements
        (regex::Regex::new(r#"SignedJWT: [^\s]+"#).unwrap(), "SignedJWT: *****"),
        (regex::Regex::new(r#"Session ID is [^\s)]+"#).unwrap(), "Session ID is *****"),
        (regex::Regex::new(r#"--accessToken, [^\s,]+"#).unwrap(), "--accessToken, *****"),
        // Computer username replacements
        (regex::Regex::new(r#"\/home\/[^/]+\/"#).unwrap(), "/home/*****/"),
        (regex::Regex::new(r#"\/Users\/[^/]+\/"#).unwrap(), "/Users/*****/"),
        (regex::Regex::new(r#"\\Users\\[^\\]+\\"#).unwrap(), "\\Users\\*****\\"),
        (regex::Regex::new(r#"\\\\Users\\\\[^/]+\\\\"#).unwrap(), "\\\\Users\\\\*****\\\\"),
    ]
});

pub fn replace(string: &str) -> Cow<'_, str> {
    let mut replaced = Cow::Borrowed(string);
    for (regex, replacement) in &*REPLACEMENTS {
        if let Cow::Owned(new) = regex.replace_all(&replaced, *replacement) {
            replaced = Cow::Owned(new);
        }
    }
    replaced
}

pub fn start_game_output(stdout: PipeReader, stderr: Option<PipeReader>, frontend: FrontendHandle) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    frontend.send(MessageToFrontend::CreateGameOutputWindow { receiver });

    if let Some(stderr) = stderr {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let mut raw_text = String::new();
            let mut reader = BufReader::new(stderr);

            loop {
                match reader.read_line(&mut raw_text) {
                    Err(e) => panic!("Error while reading stderr: {:?}", e),
                    Ok(0) => {
                        return; // EOF
                    },
                    Ok(_) => {
                        let replaced = replace(&*raw_text);
                        let replaced = replaced.trim_end();

                        #[cfg(debug_assertions)]
                        if replaced.contains('\n') {
                            panic!("Line contains newline: {replaced:?}")
                        }

                        let res = sender.send(GameOutputMsg {
                            time: Utc::now().timestamp_millis(),
                            level: GameOutputLogLevel::Error,
                            text: Arc::new([replaced.into()]),
                        });
                        if res.is_err() {
                            return; // Window closed
                        }
                        raw_text.clear();
                    },
                }
            }
        });
    }

    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut log_reader = LogReader {
            stack: Vec::new(),
            sender: sender.clone(),
            empty_message: "<empty>".into(),
        };
        let mut log_input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader,
        };

        #[cfg(debug_assertions)]
        let result = {
            let panic_result = std::panic::catch_unwind(move || log_reader.handle_output(&mut log_input));
            match panic_result {
                Ok(result) => result,
                Err(panic_error) => {
                    let panic_error_str = match panic_error.downcast::<&str>() {
                        Ok(str) => String::from(*str),
                        Err(panic_error) => match panic_error.downcast::<String>() {
                            Ok(string) => *string,
                            Err(_) => "unable to convert panic message to &str".to_string(),
                        },
                    };

                    let panic_message =
                        format!("(Pandora) There was an error while reading the log: {panic_error_str}");

                    _ = sender.send(GameOutputMsg {
                        time: Utc::now().timestamp_millis(),
                        level: GameOutputLogLevel::Fatal,
                        text: panic_message.lines().map(Arc::from).collect::<Arc<[_]>>(),
                    });
                    return;
                },
            }
        };
        #[cfg(not(debug_assertions))]
        let result = log_reader.handle_output(&mut log_input);

        if let Err(HandleOutputError::ReceiverClosed) = result {
            return;
        }

        if let Err(error) = result {
            let error_message = format!("(Pandora) There was an error while reading the log: {error}");

            _ = sender.send(GameOutputMsg {
                time: Utc::now().timestamp_millis(),
                level: GameOutputLogLevel::Fatal,
                text: error_message.lines().map(Arc::from).collect::<Arc<[_]>>(),
            });
        }
    });
}

#[derive(Error, Debug)]
enum HandleOutputError {
    #[error("An I/O error occurred:\n{0}")]
    IoError(#[from] std::io::Error),
    #[error("Unable to convert text to UTF-8:\n{0}")]
    Utf8Error(#[from] std::str::Utf8Error),
    #[error("Unexpected Eof")]
    UnexpectedEof,
    #[error("Invalid CDATA")]
    InvalidCdata,
    #[error("Invalid Comment")]
    InvalidComment,
    #[error("Unmatched element")]
    UnmatchedElement(String),
    #[error("Receiver closed")]
    ReceiverClosed,
    #[error("Invalid attribute: {0}")]
    InvalidAttribute(String),
}

struct LogReader {
    stack: Vec<LogOutputState>,
    sender: tokio::sync::mpsc::UnboundedSender<GameOutputMsg>,
    empty_message: Arc<str>,
}

struct LogInput<R: BufRead> {
    buffer: Vec<u8>,
    pending_markup: Vec<u8>,
    reader: R,
}

#[derive(Debug)]
enum LogOutputState {
    Event {
        timestamp: Option<i64>,
        level: Option<GameOutputLogLevel>,
        text: Option<Arc<str>>,
        throwable: Option<Arc<str>>,
    },
    Message {
        content: Option<Arc<str>>,
    },
    Throwable {
        content: Option<Arc<str>>,
    },
    Unknown,
}

#[derive(PartialEq, Eq)]
enum ReadAttributesForElement {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamedAttributeKey {
    Logger,
    Timestamp,
    Level,
    Thread,
    Unknown,
}

impl LogReader {
    pub fn handle_output<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        loop {
            let available = input.reader.fill_buf()?;
            if available.is_empty() {
                return Ok(());
            }

            // If we are inside XML, only try to read XML
            if !self.stack.is_empty() {
                let Some(index) = memchr::memchr(b'<', available) else {
                    let read = available.len();
                    input.reader.consume(read);
                    continue;
                };

                input.reader.consume(index + 1);
                self.read_markup(input)?;

                continue;
            }

            // Try to read either XML or a raw line
            let Some(index) = memchr::memchr2(b'\n', b'<', available) else {
                let buffer_contains_non_whitespace = !available.trim_ascii().is_empty();

                input.buffer.extend_from_slice(available);
                let read = available.len();
                input.reader.consume(read);

                if buffer_contains_non_whitespace {
                    self.read_rest_of_line(input)?;
                }

                continue;
            };

            if available[index] == b'\n' {
                self.finish_text(&available[..index], &mut input.buffer)?;
                input.reader.consume(index + 1);
            } else if !available[..index].trim_ascii().is_empty() {
                // Line contains non-whitespace before <, treat as a literal line instead of markup
                if let Some(new_index) = memchr::memchr(b'\n', &available[index..]) {
                    self.finish_text(&available[..index + new_index], &mut input.buffer)?;
                    input.reader.consume(index + new_index + 1);
                    continue;
                }

                input.buffer.extend_from_slice(available);
                let read = available.len();
                input.reader.consume(read);

                self.read_rest_of_line(input)?;
            } else {
                input.buffer.clear();
                input.reader.consume(index + 1);
                self.read_markup(input)?;
            }
        }
    }

    fn read_markup<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        let available = input.reader.fill_buf()?;
        if available.is_empty() {
            return Err(HandleOutputError::UnexpectedEof);
        }

        let peeked = available[0];
        if peeked == b'!' {
            input.reader.consume(1);
            self.read_bang(input)?;
        } else if peeked == b'/' {
            input.reader.consume(1);
            self.read_end_element(input)?;
        } else if peeked == b'?' {
            input.reader.consume(1);
            self.read_processing_instruction(input)?;
        } else {
            self.read_element(input)?;
        }

        debug_assert!(input.buffer.is_empty());
        Ok(())
    }

    fn read_bang<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        let available = input.reader.fill_buf()?;
        if available.is_empty() {
            return Err(HandleOutputError::UnexpectedEof);
        }
        match available[0] {
            b'[' => self.read_cdata(input),
            b'-' => self.read_comment(input),
            _ => Self::skip_balanced_angle_brackets(1, input),
        }
    }

    fn read_cdata<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        // consume '['
        input.reader.consume(1);
        // need "CDATA[" (6 bytes)
        let mut prefix = Vec::new();
        while prefix.len() < 6 {
            let available = input.reader.fill_buf()?;
            if available.is_empty() {
                return Err(HandleOutputError::UnexpectedEof);
            }
            let need = 6 - prefix.len();
            let take = need.min(available.len());
            prefix.extend_from_slice(&available[..take]);
            input.reader.consume(take);
        }
        if prefix != b"CDATA[" {
            return Err(HandleOutputError::InvalidCdata);
        }
        // collect until "]]>"
        let content = Self::collect_until(input, b"]]>")?;
        // prefix already validated, pass content directly
        self.apply_cdata_content(&content)?;
        Ok(())
    }

    fn read_comment<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        // consume first '-'
        input.reader.consume(1);
        let available = input.reader.fill_buf()?;
        if available.is_empty() {
            return Err(HandleOutputError::UnexpectedEof);
        }
        if available[0] != b'-' {
            return Err(HandleOutputError::InvalidComment);
        }
        input.reader.consume(1);
        Self::skip_until(input, b"-->")?;
        Ok(())
    }

    fn read_processing_instruction<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        Self::skip_until(input, b"?>")?;
        Ok(())
    }

    fn skip_balanced_angle_brackets<R: BufRead>(mut depth: usize, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        loop {
            let available = input.reader.fill_buf()?;
            if available.is_empty() {
                return Err(HandleOutputError::UnexpectedEof);
            }
            let Some(index) = memchr::memchr2(b'<', b'>', available) else {
                let read = available.len();
                input.reader.consume(read);
                continue;
            };
            let last = available[index];
            input.reader.consume(index + 1);
            if last == b'<' {
                depth += 1;
            } else {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
    }

    fn read_element<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        let tag_bytes = Self::read_tag_bytes(input)?;
        if tag_bytes.is_empty() {
            self.stack.push(LogOutputState::Unknown);
            return Ok(());
        }
        // detect self-closing
        let is_empty = {
            let mut i = tag_bytes.len();
            while i > 0 && is_xml_whitespace(tag_bytes[i - 1]) {
                i -= 1;
            }
            i > 0 && tag_bytes[i - 1] == b'/'
        };
        let content_for_parse: Vec<u8> = if is_empty {
            let mut end = tag_bytes.len();
            while end > 0 && is_xml_whitespace(tag_bytes[end - 1]) {
                end -= 1;
            }
            end -= 1; // remove '/'
            while end > 0 && is_xml_whitespace(tag_bytes[end - 1]) {
                end -= 1;
            }
            tag_bytes[..end].to_vec()
        } else {
            tag_bytes.clone()
        };
        let content_str = std::str::from_utf8(&content_for_parse).map_err(HandleOutputError::Utf8Error)?;
        let content_str = content_str.trim();
        if content_str.is_empty() {
            self.stack.push(LogOutputState::Unknown);
            if is_empty {
                self.apply_end_element(b"")?;
            }
            return Ok(());
        }
        // Compute name_len using byte positions before passing to BytesStart
        let name_len = content_str.bytes().position(|b| is_xml_whitespace(b)).unwrap_or(content_str.len());
        let bs = quick_xml::events::BytesStart::from_content(content_str, name_len);
        let name = bs.name();
        let read_attrs = self.apply_new_element(name.as_ref());
        if read_attrs == ReadAttributesForElement::Yes {
            for attr in bs.attributes().with_checks(false) {
                let attr = attr.map_err(|e| HandleOutputError::InvalidAttribute(format!("{:?}", e)))?;
                let key = match attr.key.as_ref() {
                    b"logger" => NamedAttributeKey::Logger,
                    b"timestamp" => NamedAttributeKey::Timestamp,
                    b"level" => NamedAttributeKey::Level,
                    b"thread" => NamedAttributeKey::Thread,
                    _ => NamedAttributeKey::Unknown,
                };
                let value = attr.value.as_ref();
                self.apply_attribute_key_value(key, value);
            }
        }
        if is_empty {
            self.apply_end_element(name.as_ref())?;
        }
        Ok(())
    }

    fn read_end_element<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        let tag_bytes = Self::read_tag_bytes(input)?;
        let mut end = tag_bytes.len();
        while end > 0 && is_xml_whitespace(tag_bytes[end - 1]) {
            end -= 1;
        }
        // trim leading whitespace
        let mut start = 0;
        while start < end && is_xml_whitespace(tag_bytes[start]) {
            start += 1;
        }
        // name ends at whitespace or end
        let mut name_end = start;
        while name_end < end && !is_xml_whitespace(tag_bytes[name_end]) {
            name_end += 1;
        }
        let name = &tag_bytes[start..name_end];
        self.apply_end_element(name)?;
        Ok(())
    }

    fn read_tag_bytes<R: BufRead>(input: &mut LogInput<R>) -> Result<Vec<u8>, HandleOutputError> {
        let mut out = Vec::new();
        let mut in_single = false;
        let mut in_double = false;
        loop {
            let available = input.reader.fill_buf()?;
            if available.is_empty() {
                return Err(HandleOutputError::UnexpectedEof);
            }
            let mut consumed = 0;
            for &b in available {
                if b == b'\'' && !in_double {
                    in_single = !in_single;
                } else if b == b'"' && !in_single {
                    in_double = !in_double;
                } else if b == b'>' && !in_single && !in_double {
                    if out.len() + consumed > MAX_ACCUMULATOR_SIZE {
                        return Err(HandleOutputError::UnexpectedEof);
                    }
                    out.extend_from_slice(&available[..consumed]);
                    input.reader.consume(consumed + 1);
                    return Ok(out);
                }
                consumed += 1;
            }
            if out.len() + available.len() > MAX_ACCUMULATOR_SIZE {
                return Err(HandleOutputError::UnexpectedEof);
            }
            out.extend_from_slice(available);
            input.reader.consume(available.len());
        }
    }

    fn collect_until<R: BufRead>(input: &mut LogInput<R>, needle: &[u8]) -> Result<Vec<u8>, HandleOutputError> {
        let mut accum = std::mem::take(&mut input.pending_markup);
        let finder = memchr::memmem::Finder::new(needle);
        let mut search_offset = 0;

        loop {
            // Check if needle is already in accumulated data
            let search_start = search_offset.saturating_sub(needle.len().saturating_sub(1));
            if let Some(pos) = finder.find(&accum[search_start..]) {
                let actual_pos = search_start + pos;
                let result = accum[..actual_pos].to_vec();
                let tail = accum[actual_pos + needle.len()..].to_vec();
                input.pending_markup = tail;
                return Ok(result);
            }

            search_offset = accum.len();
            let available = input.reader.fill_buf()?;
            if available.is_empty() {
                return Err(HandleOutputError::UnexpectedEof);
            }
            if accum.len() + available.len() > MAX_ACCUMULATOR_SIZE {
                return Err(HandleOutputError::UnexpectedEof);
            }
            accum.extend_from_slice(available);
            let consumed = available.len();
            input.reader.consume(consumed);
        }
    }

    fn skip_until<R: BufRead>(input: &mut LogInput<R>, needle: &[u8]) -> Result<(), HandleOutputError> {
        let _ = Self::collect_until(input, needle)?;
        Ok(())
    }

    fn apply_cdata(&mut self, cdata: &[u8]) -> Result<(), HandleOutputError> {
        let Some(cdata) = cdata.strip_prefix(b"[CDATA[") else {
            return Err(HandleOutputError::InvalidCdata);
        };
        self.apply_cdata_content(cdata)
    }

    fn apply_cdata_content(&mut self, cdata: &[u8]) -> Result<(), HandleOutputError> {
        let str = match str::from_utf8(cdata) {
            Ok(str) => Cow::Borrowed(str),
            Err(err) => Cow::Owned(format!("{}", HandleOutputError::Utf8Error(err))),
        };

        match self.stack.last_mut() {
            None => {
                self.send_raw_text(&str)?;
            },
            Some(LogOutputState::Message { content }) => {
                *content = Some(str.into());
            },
            Some(LogOutputState::Throwable { content }) => {
                *content = Some(str.into());
            },
            last => {
                if cfg!(debug_assertions) {
                    panic!("Unexpected cdata on {:?}", last);
                }
            },
        }
        Ok(())
    }

    fn apply_new_element(&mut self, name: &[u8]) -> ReadAttributesForElement {
        match self.stack.last_mut() {
            None => {
                if name == b"log4j:Event" {
                    self.stack.push(LogOutputState::Event {
                        timestamp: None,
                        level: None,
                        text: None,
                        throwable: None,
                    });
                    return ReadAttributesForElement::Yes;
                } else if cfg!(debug_assertions) {
                    panic!("Unexpected element {:?} on {:?}", str::from_utf8(name), self.stack.last_mut());
                } else {
                    self.stack.push(LogOutputState::Unknown);
                }
            },
            Some(LogOutputState::Event { .. }) => {
                if name == b"log4j:Message" {
                    self.stack.push(LogOutputState::Message { content: None });
                } else if name == b"log4j:Throwable" {
                    self.stack.push(LogOutputState::Throwable { content: None });
                } else if cfg!(debug_assertions) {
                    panic!("Unexpected element {:?} on {:?}", str::from_utf8(name), self.stack.last_mut());
                } else {
                    self.stack.push(LogOutputState::Unknown);
                }
            },
            _ => {
                if cfg!(debug_assertions) {
                    panic!("Unexpected element {:?} on {:?}", str::from_utf8(name), self.stack.last_mut());
                } else {
                    self.stack.push(LogOutputState::Unknown);
                }
            },
        }
        ReadAttributesForElement::No
    }

    fn apply_end_element(&mut self, name: &[u8]) -> Result<(), HandleOutputError> {
        match self.stack.last_mut() {
            Some(LogOutputState::Event { .. }) => {
                if name != b"log4j:Event" {
                    return Err(HandleOutputError::UnmatchedElement(str::from_utf8(name)?.into()));
                }

                let Some(LogOutputState::Event {
                    timestamp,
                    level,
                    mut text,
                    mut throwable,
                }) = self.stack.pop()
                else {
                    unreachable!()
                };
                let mut lines = Vec::new();

                if let Some(text) = text.as_mut() {
                    let replaced = replace(&**text);
                    if let Cow::Owned(replaced) = replaced {
                        *text = replaced.into();
                    }
                }
                if let Some(throwable) = throwable.as_mut() {
                    let replaced = replace(&**throwable);
                    if let Cow::Owned(replaced) = replaced {
                        *throwable = replaced.into();
                    }
                }

                if let Some(text) = &text {
                    let mut split = text.split('\n');
                    if let Some(first) = split.next()
                        && let Some(second) = split.next()
                    {
                        lines.push(Arc::from(first.trim_end()));
                        lines.push(Arc::from(second.trim_end()));
                        for next in split {
                            lines.push(Arc::from(next.trim_end()));
                        }
                    }
                }
                if let Some(throwable) = &throwable {
                    let mut split = throwable.split('\n');
                    if let Some(first) = split.next()
                        && let Some(second) = split.next()
                    {
                        if let Some(text) = text.take()
                            && lines.is_empty()
                        {
                            lines.push(text);
                        }

                        lines.push(Arc::from(first.trim_end()));
                        lines.push(Arc::from(second.trim_end()));
                        for next in split {
                            lines.push(Arc::from(next.trim_end()));
                        }
                    }
                }

                let final_lines: Arc<[Arc<str>]> = if !lines.is_empty() {
                    lines.into()
                } else if let Some(text) = text.take() {
                    if let Some(throwable) = throwable.take() {
                        Arc::new([text, throwable])
                    } else {
                        Arc::new([text])
                    }
                } else if let Some(throwable) = throwable {
                    Arc::new([throwable])
                } else {
                    Arc::new([self.empty_message.clone()])
                };
                let res = self.sender.send(GameOutputMsg {
                    time: timestamp.unwrap_or(Utc::now().timestamp_millis()),
                    level: level.unwrap_or(GameOutputLogLevel::Other),
                    text: final_lines,
                });
                if res.is_err() {
                    return Err(HandleOutputError::ReceiverClosed);
                }
            },
            Some(LogOutputState::Message { .. }) => {
                if name != b"log4j:Message" {
                    return Err(HandleOutputError::UnmatchedElement(str::from_utf8(name)?.into()));
                }

                let Some(LogOutputState::Message { content }) = self.stack.pop() else {
                    unreachable!()
                };

                if let Some(LogOutputState::Event { text, .. }) = self.stack.last_mut() {
                    *text = content;
                } else {
                    panic!("log4j:Message should only be inside log4j:Event");
                }
            },
            Some(LogOutputState::Throwable { .. }) => {
                if name != b"log4j:Throwable" {
                    return Err(HandleOutputError::UnmatchedElement(str::from_utf8(name)?.into()));
                }

                let Some(LogOutputState::Throwable { content }) = self.stack.pop() else {
                    unreachable!()
                };

                if let Some(LogOutputState::Event { throwable, .. }) = self.stack.last_mut() {
                    *throwable = content;
                } else {
                    panic!("log4j:Throwable should only be inside log4j:Event");
                }
            },
            Some(LogOutputState::Unknown) => {
                _ = self.stack.pop();
            },
            None => {
                return Err(HandleOutputError::UnmatchedElement(str::from_utf8(name)?.into()));
            },
        }
        Ok(())
    }

    fn apply_attribute_key_value(&mut self, key: NamedAttributeKey, value: &[u8]) {
        match self.stack.last_mut() {
            Some(LogOutputState::Event { timestamp, level, .. }) => {
                match key {
                    NamedAttributeKey::Logger => {
                        // Ignore
                    },
                    NamedAttributeKey::Timestamp => {
                        let Ok(value) = str::from_utf8(&value) else {
                            return;
                        };
                        if let Ok(parsed) = value.parse() {
                            *timestamp = Some(parsed);
                        }
                    },
                    NamedAttributeKey::Level => {
                        *level = Some(match value {
                            b"FATAL" => GameOutputLogLevel::Fatal,
                            b"ERROR" => GameOutputLogLevel::Error,
                            b"WARN" => GameOutputLogLevel::Warn,
                            b"INFO" => GameOutputLogLevel::Info,
                            b"DEBUG" => GameOutputLogLevel::Debug,
                            b"TRACE" => GameOutputLogLevel::Trace,
                            _ => GameOutputLogLevel::Other,
                        });
                    },
                    NamedAttributeKey::Thread => {
                        // Ignore
                    },
                    _ => {
                        if cfg!(debug_assertions) {
                            panic!("Unexpected attribute {:?} on {:?}", key, self.stack.last_mut());
                        }
                    },
                }
            },
            _ => {
                if cfg!(debug_assertions) {
                    panic!("Unexpected attribute {:?} on {:?}", key, self.stack.last_mut());
                }
            },
        }
    }

    fn read_rest_of_line<R: BufRead>(&mut self, input: &mut LogInput<R>) -> Result<(), HandleOutputError> {
        loop {
            let available = input.reader.fill_buf()?;

            if available.is_empty() {
                self.finish_text(b"", &mut input.buffer)?;
                return Ok(());
            }

            if let Some(index) = memchr::memchr(b'\n', available) {
                self.finish_text(&available[..index], &mut input.buffer)?;
                return Ok(());
            } else {
                input.buffer.extend_from_slice(available);
                let read = available.len();
                input.reader.consume(read);
            }
        }
    }

    fn finish_text(&mut self, remaining: &[u8], buffer: &mut Vec<u8>) -> Result<(), HandleOutputError> {
        let line = if buffer.is_empty() {
            str::from_utf8(remaining)
        } else {
            buffer.extend_from_slice(remaining);
            str::from_utf8(&buffer)
        };

        let result = match line {
            Ok(str) => self.send_raw_text(&str),
            Err(err) => self.send_raw_text(&format!("{}", HandleOutputError::Utf8Error(err))),
        };

        buffer.clear();

        result
    }

    fn send_raw_text(&mut self, text: &str) -> Result<(), HandleOutputError> {
        if text.trim_ascii().is_empty() {
            return Ok(());
        }

        let res = self.sender.send(GameOutputMsg {
            time: Utc::now().timestamp_millis(),
            level: GameOutputLogLevel::Info,
            text: text.lines().map(Arc::from).collect::<Arc<[_]>>(),
        });
        if res.is_err() {
            return Err(HandleOutputError::ReceiverClosed);
        }

        Ok(())
    }
}

fn is_xml_whitespace(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n' | b'\t' | b' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A reader that forces small chunks to test buffer boundary conditions
    struct ChunkedReader<'a> {
        data: &'a [u8],
        pos: usize,
        chunk_size: usize,
    }

    impl<'a> ChunkedReader<'a> {
        fn new(data: &'a [u8], chunk_size: usize) -> Self {
            Self { data, pos: 0, chunk_size }
        }
    }

    impl<'a> Read for ChunkedReader<'a> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let remaining = self.data.len() - self.pos;
            let to_read = remaining.min(self.chunk_size).min(buf.len());
            buf[..to_read].copy_from_slice(&self.data[self.pos..self.pos + to_read]);
            self.pos += to_read;
            Ok(to_read)
        }
    }

    fn run(data: &[u8]) -> Vec<GameOutputMsg> {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let cursor = std::io::Cursor::new(data);
        let mut input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader: cursor,
        };
        let mut reader = LogReader {
            stack: Vec::new(),
            sender,
            empty_message: "<empty>".into(),
        };
        reader.handle_output(&mut input).unwrap();
        drop(reader);
        let mut msgs = Vec::new();
        while let Ok(msg) = receiver.try_recv() {
            msgs.push(msg);
        }
        msgs
    }

    #[test]
    fn plain_text() {
        let msgs = run(b"hello world\n");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "hello world");
        assert_eq!(msgs[0].level, GameOutputLogLevel::Info);
    }

    #[test]
    fn xml_event_simple() {
        let data = br#"<log4j:Event logger="test" timestamp="1234567890123" level="INFO" thread="main"><log4j:Message><![CDATA[hello from xml]]></log4j:Message></log4j:Event>"#;
        let mut d = Vec::new();
        d.extend_from_slice(data);
        d.push(b'\n');
        let msgs = run(&d);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "hello from xml");
        assert_eq!(msgs[0].level, GameOutputLogLevel::Info);
        assert_eq!(msgs[0].time, 1234567890123);
    }

    #[test]
    fn xml_with_throwable() {
        let data = br#"<log4j:Event logger="x" timestamp="1" level="ERROR" thread="t"><log4j:Message><![CDATA[msg line]]></log4j:Message><log4j:Throwable><![CDATA[throwable line]]></log4j:Throwable></log4j:Event>"#;
        let mut d = Vec::new();
        d.extend_from_slice(data);
        d.push(b'\n');
        let msgs = run(&d);
        assert_eq!(msgs.len(), 1);
        // original logic: if text has only one line, it keeps text+throwable as two entries
        // our sample has single line each, so should be two lines
        assert_eq!(msgs[0].text.len(), 2);
    }

    #[test]
    fn redaction() {
        let data = br#"<log4j:Event logger="x" timestamp="1" level="INFO" thread="t"><log4j:Message><![CDATA[SignedJWT: secret123]]></log4j:Message></log4j:Event>"#;
        let mut d = Vec::new();
        d.extend_from_slice(data);
        d.push(b'\n');
        let msgs = run(&d);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "SignedJWT: *****");
    }

    #[test]
    fn mixed_plain_and_xml() {
        let data = b"plain line\n<log4j:Event logger=\"a\" timestamp=\"1\" level=\"WARN\" thread=\"t\"><log4j:Message><![CDATA[xml line]]></log4j:Message></log4j:Event>\nanother plain\n";
        let msgs = run(data);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].text[0].as_ref(), "plain line");
        assert_eq!(msgs[1].text[0].as_ref(), "xml line");
        assert_eq!(msgs[1].level, GameOutputLogLevel::Warn);
        assert_eq!(msgs[2].text[0].as_ref(), "another plain");
    }

    #[test]
    fn comment_and_pi_ignored() {
        let data = b"<?xml version=\"1.0\"?><!-- comment --><log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"t\"><log4j:Message><![CDATA[hi]]></log4j:Message></log4j:Event>\n";
        let msgs = run(data);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "hi");
    }

    #[test]
    fn stray_angle_bracket_as_plain() {
        let data = b"a < b\n";
        let msgs = run(data);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "a < b");
    }

    #[test]
    fn cdata_split_across_chunks() {
        // Use ChunkedReader to force buffer boundaries through collect_until
        let data = br#"<log4j:Event logger="a" timestamp="1" level="INFO" thread="t"><log4j:Message><![CDATA[split content]]></log4j:Message></log4j:Event>"#;
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let chunked = ChunkedReader::new(data, 5); // Small chunks to test boundaries
        let mut input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader: std::io::BufReader::new(chunked),
        };
        let mut reader = LogReader {
            stack: Vec::new(),
            sender,
            empty_message: "<empty>".into(),
        };
        reader.handle_output(&mut input).unwrap();
        drop(reader);
        let mut msgs = Vec::new();
        while let Ok(msg) = receiver.try_recv() {
            msgs.push(msg);
        }
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "split content");
    }

    #[test]
    fn delimiter_split_across_chunks() {
        // Test splitting across ]]>, -->, and ?> delimiters
        let data = b"<?xml version=\"1.0\"?><!-- test comment --><log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"t\"><log4j:Message><![CDATA[content]]></log4j:Message></log4j:Event>";
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let chunked = ChunkedReader::new(data, 3); // Very small chunks to split delimiters
        let mut input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader: std::io::BufReader::new(chunked),
        };
        let mut reader = LogReader {
            stack: Vec::new(),
            sender,
            empty_message: "<empty>".into(),
        };
        reader.handle_output(&mut input).unwrap();
        drop(reader);
        let mut msgs = Vec::new();
        while let Ok(msg) = receiver.try_recv() {
            msgs.push(msg);
        }
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text[0].as_ref(), "content");
    }

    #[test]
    fn self_closing_event() {
        let data = br#"<log4j:Event logger="a" timestamp="1" level="WARN" thread="t" />"#;
        let msgs = run(data);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].level, GameOutputLogLevel::Warn);
        assert_eq!(msgs[0].text[0].as_ref(), "<empty>");
    }

    #[test]
    fn invalid_cdata_prefix() {
        let data = b"<log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"t\"><log4j:Message><![XDATA[bad]]></log4j:Message></log4j:Event>";
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let cursor = std::io::Cursor::new(data);
        let mut input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader: cursor,
        };
        let mut reader = LogReader {
            stack: Vec::new(),
            sender,
            empty_message: "<empty>".into(),
        };
        let result = reader.handle_output(&mut input);
        assert!(matches!(result, Err(HandleOutputError::InvalidCdata)));
        drop(receiver);
    }

    #[test]
    fn truncated_input() {
        let data = b"<log4j:Event logger=\"a\" timestamp=\"1\" level=\"INFO\" thread=\"t\"><log4j:Message><![CDATA[incomplete";
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let cursor = std::io::Cursor::new(data);
        let mut input = LogInput {
            buffer: Vec::new(),
            pending_markup: Vec::new(),
            reader: cursor,
        };
        let mut reader = LogReader {
            stack: Vec::new(),
            sender,
            empty_message: "<empty>".into(),
        };
        let result = reader.handle_output(&mut input);
        assert!(matches!(result, Err(HandleOutputError::UnexpectedEof)));
        drop(receiver);
    }
}
