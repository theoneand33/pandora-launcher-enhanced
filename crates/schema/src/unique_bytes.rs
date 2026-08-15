use std::{
    borrow::Cow,
    ops::Deref,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize, de::Visitor};
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UniqueBytes(Arc<[u8]>);

static UNIQUE: LazyLock<Mutex<FxHashMap<Vec<u8>, Weak<[u8]>>>> = LazyLock::new(Default::default);

impl Deref for UniqueBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&[u8]> for UniqueBytes {
    fn from(value: &[u8]) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u8>> for UniqueBytes {
    fn from(value: Vec<u8>) -> Self {
        Self::new(&value)
    }
}

impl From<Cow<'_, [u8]>> for UniqueBytes {
    fn from(value: Cow<'_, [u8]>) -> Self {
        Self::new(value.as_ref())
    }
}

impl UniqueBytes {
    pub fn new(bytes: &[u8]) -> UniqueBytes {
        let mut map = UNIQUE.lock();
        if let Some(weak) = map.get(bytes) {
            if let Some(arc) = weak.upgrade() {
                return UniqueBytes(arc);
            }
            map.remove(bytes);
        }

        let arc: Arc<[u8]> = Arc::from(bytes);
        map.insert(bytes.to_vec(), Arc::downgrade(&arc));
        UniqueBytes(arc)
    }
}

impl Serialize for UniqueBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&**self)
    }
}

impl<'de> Deserialize<'de> for UniqueBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_bytes(UniqueBytesVisitor)
    }
}

struct UniqueBytesVisitor;

impl<'de> Visitor<'de> for UniqueBytesVisitor {
    type Value = UniqueBytes;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("bytes")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let capacity = seq.size_hint().unwrap_or(0).max(0);
        let mut values = Vec::<u8>::with_capacity(capacity);

        while let Some(element) = seq.next_element()? {
            values.push(element);
        }

        Ok(UniqueBytes::new(&values))
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(UniqueBytes::new(v))
    }
}
