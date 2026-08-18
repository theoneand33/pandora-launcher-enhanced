pub use reference::{CompoundRef, CompoundRefMut, ListRef, ListRefMut, NBTRef, NBTRefMut};
use slab::Slab;
use std::{fmt::Debug, result};

pub mod decode;
pub mod encode;

mod reference;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TagType(pub(crate) u8);

pub const TAG_END_ID: TagType = TagType(0);
pub const TAG_BYTE_ID: TagType = TagType(1);
pub const TAG_SHORT_ID: TagType = TagType(2);
pub const TAG_INT_ID: TagType = TagType(3);
pub const TAG_LONG_ID: TagType = TagType(4);
pub const TAG_FLOAT_ID: TagType = TagType(5);
pub const TAG_DOUBLE_ID: TagType = TagType(6);
pub const TAG_BYTE_ARRAY_ID: TagType = TagType(7);
pub const TAG_STRING_ID: TagType = TagType(8);
pub const TAG_LIST_ID: TagType = TagType(9);
pub const TAG_COMPOUND_ID: TagType = TagType(10);
pub const TAG_INT_ARRAY_ID: TagType = TagType(11);
pub const TAG_LONG_ARRAY_ID: TagType = TagType(12);

#[derive(Clone)]
pub struct NBT {
    pub root_name: String,
    root_index: usize,
    nodes: Slab<NBTNode>,
}

impl Default for NBT {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for NBT {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NBT")
            .field("root_name", &self.root_name)
            .field("root", &self.as_reference())
            .finish()
    }
}

impl PartialEq for NBT {
    fn eq(&self, other: &Self) -> bool {
        self.as_reference() == other.as_reference()
    }
}

macro_rules! insert {
    ($name:ident, $value_type:ty, $node:ident) => {
        paste::paste! {
            pub fn [<insert_ $name>](&mut self, key: &str, value: $value_type) {
                self.insert_node(key, NBTNode::$node(value));
            }
        }
    };
}

macro_rules! find {
    ($name:ident, $value_type:ty, $node:ident) => {
        paste::paste! {
            pub fn [<find_ $name>](&self, key: &str) -> Option<&$value_type> {
                let idx = self.find_idx(key)?;
                match self.get_node(idx) {
                    NBTNode::$node(value) => Some(value),
                    _ => None
                }
            }
        }
    };
}

macro_rules! enumerate_basic_types {
    ($macro:path) => {
        $macro!(byte, i8, Byte);
        $macro!(short, i16, Short);
        $macro!(int, i32, Int);
        $macro!(long, i64, Long);
        $macro!(float, f32, Float);
        $macro!(double, f64, Double);
        $macro!(byte_array, Vec<i8>, ByteArray);
        $macro!(string, String, String);
        $macro!(int_array, Vec<i32>, IntArray);
        $macro!(long_array, Vec<i64>, LongArray);
    };
}

pub(crate) use enumerate_basic_types;
pub(crate) use find;
pub(crate) use insert;

impl NBT {
    pub fn new() -> NBT {
        Self::new_named(String::new())
    }

    pub fn new_named(root_name: String) -> NBT {
        let mut nodes = Slab::new();
        let root_index = nodes.insert(NBTNode::Compound(NBTCompound(Vec::new())));
        NBT {
            root_name,
            root_index,
            nodes,
        }
    }

    pub fn as_compound(&self) -> Option<CompoundRef<'_>> {
        match &self.nodes[self.root_index] {
            NBTNode::Compound(_) => Some(CompoundRef {
                nbt: self,
                node_idx: self.root_index,
            }),
            _ => None,
        }
    }

    pub fn as_compound_mut(&mut self) -> Option<CompoundRefMut<'_>> {
        match &self.nodes[self.root_index] {
            NBTNode::Compound(_) => {
                let node_idx = self.root_index;
                Some(CompoundRefMut { nbt: self, node_idx })
            },
            _ => None,
        }
    }

    pub fn as_reference(&self) -> NBTRef<'_> {
        self.get_reference(self.root_index)
    }

    pub(crate) fn remove_node(&mut self, idx: usize) {
        if idx == self.root_index {
            panic!("Cannot remove root node");
        }
        match self.nodes.remove(idx) {
            NBTNode::List { type_id: _, children } => {
                for child in children {
                    self.remove_node(child);
                }
            },
            NBTNode::Compound(compound) => {
                for (_, child) in compound.0 {
                    self.remove_node(child);
                }
            },
            _ => {},
        }
    }

    fn get_reference(&self, node_idx: usize) -> NBTRef<'_> {
        match &self.nodes[node_idx] {
            NBTNode::Byte(value) => NBTRef::Byte(value),
            NBTNode::Short(value) => NBTRef::Short(value),
            NBTNode::Int(value) => NBTRef::Int(value),
            NBTNode::Long(value) => NBTRef::Long(value),
            NBTNode::Float(value) => NBTRef::Float(value),
            NBTNode::Double(value) => NBTRef::Double(value),
            NBTNode::ByteArray(value) => NBTRef::ByteArray(value),
            NBTNode::String(value) => NBTRef::String(value),
            NBTNode::List {
                type_id: _,
                children: _,
            } => NBTRef::List(ListRef { nbt: self, node_idx }),
            NBTNode::Compound(_) => NBTRef::Compound(CompoundRef { nbt: self, node_idx }),
            NBTNode::IntArray(value) => NBTRef::IntArray(value),
            NBTNode::LongArray(value) => NBTRef::LongArray(value),
        }
    }
}

#[derive(Debug, Clone)]
enum NBTNode {
    // 32 bytes
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List {
        type_id: TagType,
        children: Vec<usize>,
    },
    Compound(NBTCompound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl NBTNode {
    pub fn get_type(&self) -> TagType {
        match self {
            NBTNode::Byte(_) => TAG_BYTE_ID,
            NBTNode::Short(_) => TAG_SHORT_ID,
            NBTNode::Int(_) => TAG_INT_ID,
            NBTNode::Long(_) => TAG_LONG_ID,
            NBTNode::Float(_) => TAG_FLOAT_ID,
            NBTNode::Double(_) => TAG_DOUBLE_ID,
            NBTNode::ByteArray(_) => TAG_BYTE_ARRAY_ID,
            NBTNode::String(_) => TAG_STRING_ID,
            NBTNode::List {
                type_id: _,
                children: _,
            } => TAG_LIST_ID,
            NBTNode::Compound(_) => TAG_COMPOUND_ID,
            NBTNode::IntArray(_) => TAG_INT_ARRAY_ID,
            NBTNode::LongArray(_) => TAG_LONG_ARRAY_ID,
        }
    }
}

// Note: Using SmartString instead of String results in worse perf
#[derive(Debug, Clone, Default)]
pub(crate) struct NBTCompound(Vec<(String, usize)>);

impl NBTCompound {
    fn find(&self, key: &str) -> Option<usize> {
        /*if self.0.len() < 8 {
            for (name, idx) in &self.0 {
                if name.as_str() == key {
                    return Some(*idx);
                }
            }
            return None;
        }*/

        match self.binary_search(key) {
            Ok(index) => Some(self.0[index].1),
            Err(_) => None,
        }
    }

    fn insert(&mut self, key: &str, value: usize) {
        match self.binary_search(key) {
            Ok(index) => {
                let _ = std::mem::replace(&mut self.0[index].1, value);
            },
            Err(index) => {
                self.0.insert(index, (key.into(), value));
            },
        }
    }

    fn binary_search(&self, key: &str) -> result::Result<usize, usize> {
        self.0.binary_search_by_key(&key, |v| v.0.as_str())
    }
}
