use std::{fmt::Debug, hint::unreachable_unchecked};

use super::{NBT, NBTCompound, NBTNode, TagType};

#[derive(Copy, Clone, Debug)]
pub enum NBTRef<'a> {
    Byte(&'a i8),
    Short(&'a i16),
    Int(&'a i32),
    Long(&'a i64),
    Float(&'a f32),
    Double(&'a f64),
    ByteArray(&'a Vec<i8>),
    String(&'a String),
    List(ListRef<'a>),
    Compound(CompoundRef<'a>),
    IntArray(&'a Vec<i32>),
    LongArray(&'a Vec<i64>),
}

impl PartialEq for NBTRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Byte(l0), Self::Byte(r0)) => l0 == r0,
            (Self::Short(l0), Self::Short(r0)) => l0 == r0,
            (Self::Int(l0), Self::Int(r0)) => l0 == r0,
            (Self::Long(l0), Self::Long(r0)) => l0 == r0,
            (Self::Float(l0), Self::Float(r0)) => l0 == r0 || (l0.is_nan() && r0.is_nan()),
            (Self::Double(l0), Self::Double(r0)) => l0 == r0 || (l0.is_nan() && r0.is_nan()),
            (Self::ByteArray(l0), Self::ByteArray(r0)) => l0 == r0,
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::List(l0), Self::List(r0)) => l0 == r0,
            (Self::Compound(l0), Self::Compound(r0)) => l0 == r0,
            (Self::IntArray(l0), Self::IntArray(r0)) => l0 == r0,
            (Self::LongArray(l0), Self::LongArray(r0)) => l0 == r0,
            _ => false,
        }
    }
}

impl<'a> NBTRef<'a> {
    pub fn as_compound(self) -> Option<CompoundRef<'a>> {
        match self {
            NBTRef::Compound(compound) => Some(compound),
            _ => None,
        }
    }
}

#[derive(Copy, Clone)]
pub struct CompoundRef<'a> {
    pub(crate) nbt: &'a NBT,
    pub(crate) node_idx: usize,
}

impl<'a> Debug for CompoundRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.entries().map(|(k, v)| (k, format!("{:?}", v)))).finish()
    }
}

impl PartialEq for CompoundRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        let self_compound = self.get_self_node();
        let other_compound = other.get_self_node();

        if self_compound.0.len() != other_compound.0.len() {
            return false;
        }

        let zipped = self_compound.0.iter().zip(other_compound.0.iter());
        for ((self_child_name, self_child_idx), (other_child_name, other_child_idx)) in zipped {
            if self_child_name != other_child_name {
                return false;
            }
            if self.nbt.get_reference(*self_child_idx) != other.nbt.get_reference(*other_child_idx) {
                return false;
            }
        }

        true
    }
}

impl<'a> CompoundRef<'a> {
    pub(crate) fn get_self_node(&self) -> &NBTCompound {
        match self.nbt.nodes.get(self.node_idx) {
            Some(NBTNode::Compound(compound)) => compound,
            _ => unsafe { unreachable_unchecked() },
        }
    }

    fn find_idx(&self, key: &str) -> Option<usize> {
        let compound = self.get_self_node();
        compound.find(key)
    }

    fn get_node(&self, idx: usize) -> &NBTNode {
        &self.nbt.nodes[idx]
    }

    pub fn entries(&self) -> CompoundIterator<'_> {
        CompoundIterator {
            nbt: self.nbt,
            compound: self.get_self_node(),
            index: 0,
        }
    }

    super::enumerate_basic_types!(super::find);

    pub fn find_numeric<T: num_traits::FromPrimitive>(&self, key: &str) -> Option<T> {
        let idx = self.find_idx(key)?;
        match self.get_node(idx) {
            NBTNode::Byte(v) => T::from_i8(*v),
            NBTNode::Short(v) => T::from_i16(*v),
            NBTNode::Int(v) => T::from_i32(*v),
            NBTNode::Long(v) => T::from_i64(*v),
            NBTNode::Float(v) => T::from_f32(*v),
            NBTNode::Double(v) => T::from_f64(*v),
            NBTNode::ByteArray(_) => None,
            NBTNode::String(_) => None,
            NBTNode::List {
                type_id: _,
                children: _,
            } => None,
            NBTNode::Compound(_) => None,
            NBTNode::IntArray(_) => None,
            NBTNode::LongArray(_) => None,
        }
    }

    pub fn find_list(&self, key: &str, type_id: TagType) -> Option<ListRef<'_>> {
        let idx = self.find_idx(key)?;
        match self.get_node(idx) {
            NBTNode::List {
                type_id: list_type_id,
                children: _,
            } if *list_type_id == type_id => Some(ListRef {
                nbt: self.nbt,
                node_idx: idx,
            }),
            _ => None,
        }
    }

    pub fn find_compound(&self, key: &str) -> Option<CompoundRef<'_>> {
        let idx = self.find_idx(key)?;
        match self.get_node(idx) {
            NBTNode::Compound(_) => Some(CompoundRef {
                nbt: self.nbt,
                node_idx: idx,
            }),
            _ => None,
        }
    }
}

pub struct CompoundRefMut<'a> {
    pub(crate) nbt: &'a mut NBT,
    pub(crate) node_idx: usize,
}

impl<'a> Debug for CompoundRefMut<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.entries().map(|(k, v)| (k, format!("{:?}", v)))).finish()
    }
}

impl<'a> CompoundRefMut<'a> {
    pub(crate) fn get_self_node(&self) -> &NBTCompound {
        match self.nbt.nodes.get(self.node_idx) {
            Some(NBTNode::Compound(compound)) => compound,
            _ => unsafe { unreachable_unchecked() },
        }
    }

    fn get_self_node_mut(&mut self) -> &mut NBTCompound {
        match self.nbt.nodes.get_mut(self.node_idx) {
            Some(NBTNode::Compound(compound)) => compound,
            _ => unsafe { unreachable_unchecked() },
        }
    }

    fn insert_node(&mut self, key: &str, node: NBTNode) -> usize {
        let idx = self.nbt.nodes.insert(node);

        let compound = self.get_self_node_mut();
        compound.insert(key, idx);

        idx
    }

    fn find_idx(&self, key: &str) -> Option<usize> {
        let compound = self.get_self_node();
        compound.find(key)
    }

    fn get_node(&self, idx: usize) -> &NBTNode {
        &self.nbt.nodes[idx]
    }

    pub fn entries(&self) -> CompoundIterator<'_> {
        CompoundIterator {
            nbt: self.nbt,
            compound: self.get_self_node(),
            index: 0,
        }
    }

    super::enumerate_basic_types!(super::insert);

    pub fn create_list(&mut self, key: &str, type_id: TagType) -> ListRefMut<'_> {
        let idx = self.insert_node(
            key,
            NBTNode::List {
                type_id,
                children: Default::default(),
            },
        );

        ListRefMut {
            nbt: self.nbt,
            node_idx: idx,
        }
    }

    pub fn create_compound(&mut self, key: &str) -> CompoundRefMut<'_> {
        let idx = self.insert_node(key, NBTNode::Compound(Default::default()));

        CompoundRefMut {
            nbt: self.nbt,
            node_idx: idx,
        }
    }

    pub fn find_list_mut(&mut self, key: &str, type_id: TagType) -> Option<ListRefMut<'_>> {
        let idx = self.find_idx(key)?;
        match self.get_node(idx) {
            NBTNode::List {
                type_id: list_type_id,
                children: _,
            } if *list_type_id == type_id => Some(ListRefMut {
                nbt: self.nbt,
                node_idx: idx,
            }),
            _ => None,
        }
    }
}

#[derive(Copy, Clone)]
pub struct ListRef<'a> {
    pub(crate) nbt: &'a NBT,
    pub(crate) node_idx: usize,
}

impl<'a> Debug for ListRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for ListRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (self_type, self_children) = self.get_self_node();
        let (other_type, other_children) = other.get_self_node();

        if self_type != other_type || self_children.len() != other_children.len() {
            return false;
        }

        let zipped = self_children.iter().zip(other_children.iter());
        for (self_child, other_child) in zipped {
            if self.nbt.get_reference(*self_child) != other.nbt.get_reference(*other_child) {
                return false;
            }
        }

        true
    }
}

impl<'a> ListRef<'a> {
    pub(crate) fn get_self_node(&self) -> (TagType, &Vec<usize>) {
        match self.nbt.nodes.get(self.node_idx) {
            Some(NBTNode::List { type_id, children }) => (*type_id, children),
            _ => unsafe { unreachable_unchecked() },
        }
    }

    pub fn len(&self) -> usize {
        self.get_self_node().1.len()
    }

    pub fn get(&self, index: usize) -> Option<NBTRef<'_>> {
        let (_, children) = self.get_self_node();
        let idx = children.get(index)?;
        Some(self.nbt.get_reference(*idx))
    }

    pub fn iter(&self) -> ListIterator<'_> {
        ListIterator {
            nbt: self.nbt,
            indices: self.get_self_node().1,
            index: 0,
        }
    }
}

pub struct ListRefMut<'a> {
    pub(crate) nbt: &'a mut NBT,
    pub(crate) node_idx: usize,
}

impl<'a> Debug for ListRefMut<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ponytail: debug via immutable view, avoids duplicating pretty module
        let (ty, children) = self.get_self_node();
        f.debug_struct("ListRefMut").field("type", &ty.0).field("len", &children.len()).finish()
    }
}

impl<'a> ListRefMut<'a> {
    pub(crate) fn get_self_node(&self) -> (TagType, &Vec<usize>) {
        match self.nbt.nodes.get(self.node_idx) {
            Some(NBTNode::List { type_id, children }) => (*type_id, children),
            _ => unsafe { unreachable_unchecked() },
        }
    }

    fn get_self_node_mut(&mut self) -> (TagType, &mut Vec<usize>) {
        match self.nbt.nodes.get_mut(self.node_idx) {
            Some(NBTNode::List { type_id, children }) => (*type_id, children),
            _ => unsafe { unreachable_unchecked() },
        }
    }

    fn insert_node(&mut self, node: NBTNode) -> usize {
        let (type_id, _) = self.get_self_node_mut();
        if type_id != node.get_type() {
            panic!("Tried to insert {:?} into a list of {:?}", node.get_type(), type_id);
        }

        let idx = self.nbt.nodes.insert(node);
        self.get_self_node_mut().1.push(idx);
        idx
    }

    pub fn len(&self) -> usize {
        self.get_self_node().1.len()
    }

    pub fn remove_index(&mut self, index: usize) -> bool {
        let (_, children) = self.get_self_node_mut();
        if index >= children.len() {
            return false;
        }
        let idx = children.remove(index);
        self.nbt.remove_node(idx);
        true
    }

    pub fn move_index(&mut self, from: usize, to: usize) -> bool {
        let (_, children) = self.get_self_node_mut();
        if from >= children.len() || to >= children.len() || from == to {
            return false;
        }

        let entry = children.remove(from);
        let mut insert_at = to;
        if insert_at > children.len() {
            insert_at = children.len();
        }
        children.insert(insert_at, entry);
        true
    }

    pub fn get(&self, index: usize) -> Option<NBTRef<'_>> {
        let (_, children) = self.get_self_node();
        let idx = children.get(index)?;
        Some(self.nbt.get_reference(*idx))
    }

    pub fn create_compound(&mut self) -> CompoundRefMut<'_> {
        let idx = self.insert_node(NBTNode::Compound(Default::default()));

        CompoundRefMut {
            nbt: self.nbt,
            node_idx: idx,
        }
    }

    pub fn create_list(&mut self, type_id: TagType) -> ListRefMut<'_> {
        let idx = self.insert_node(NBTNode::List {
            type_id,
            children: Default::default(),
        });

        ListRefMut {
            nbt: self.nbt,
            node_idx: idx,
        }
    }
}

pub struct ListIterator<'a> {
    nbt: &'a NBT,
    indices: &'a [usize],
    index: usize,
}

impl<'a> Iterator for ListIterator<'a> {
    type Item = NBTRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.indices.len() {
            None
        } else {
            let next = self.nbt.get_reference(self.indices[self.index]);
            self.index += 1;
            Some(next)
        }
    }
}

pub struct CompoundIterator<'a> {
    nbt: &'a NBT,
    compound: &'a NBTCompound,
    index: usize,
}

impl<'a> Iterator for CompoundIterator<'a> {
    type Item = (&'a str, NBTRef<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.compound.0.len() {
            None
        } else {
            let entry = &self.compound.0[self.index];
            let next = self.nbt.get_reference(entry.1);
            self.index += 1;
            Some((&entry.0, next))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{NBT, TAG_COMPOUND_ID};

    fn make_servers_nbt(count: usize) -> NBT {
        let mut nbt = NBT::new_named("".into());
        let mut root = nbt.as_compound_mut().unwrap();
        let mut list = root.create_list("servers", TAG_COMPOUND_ID);
        for i in 0..count {
            let mut entry = list.create_compound();
            entry.insert_string("ip", format!("192.168.0.{i}"));
            entry.insert_string("name", format!("server{i}"));
        }
        nbt
    }

    #[test]
    fn remove_index_basic() {
        let mut nbt = make_servers_nbt(3);
        let mut root = nbt.as_compound_mut().unwrap();
        let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.remove_index(1));
        assert_eq!(list.len(), 2);
        // remaining should be server0 and server2
        assert_eq!(list.get(0).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server0");
        assert_eq!(list.get(1).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server2");
        drop(list);
        drop(root);
        // encode/decode round-trip still works
        let bytes = crate::encode::write_named(&nbt);
        let decoded = crate::decode::read_named(&mut bytes.as_slice()).unwrap();
        let root = decoded.as_compound().unwrap();
        let servers = root.find_list("servers", TAG_COMPOUND_ID).unwrap();
        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn remove_index_out_of_range_false() {
        let mut nbt = make_servers_nbt(2);
        let mut root = nbt.as_compound_mut().unwrap();
        let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
        assert!(!list.remove_index(5));
        assert!(!list.remove_index(2));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn remove_index_subtree_cleanup() {
        // each server entry is a compound with its own children; removing should delete subtree
        let mut nbt = make_servers_nbt(2);
        {
            let mut root = nbt.as_compound_mut().unwrap();
            let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
            assert!(list.remove_index(0));
            assert_eq!(list.len(), 1);
        }
        // after removal we can add new entries without slab corruption
        {
            let mut root = nbt.as_compound_mut().unwrap();
            let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
            let mut entry = list.create_compound();
            entry.insert_string("ip", "10.0.0.1".into());
            entry.insert_string("name", "new".into());
            assert_eq!(list.len(), 2);
        }
        let bytes = crate::encode::write_named(&nbt);
        let decoded = crate::decode::read_named(&mut bytes.as_slice()).unwrap();
        let root = decoded.as_compound().unwrap();
        let servers = root.find_list("servers", TAG_COMPOUND_ID).unwrap();
        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn move_index_reorder() {
        let mut nbt = make_servers_nbt(3);
        let mut root = nbt.as_compound_mut().unwrap();
        let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
        assert!(list.move_index(0, 2));
        // order should be 1,2,0
        assert_eq!(list.get(0).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server1");
        assert_eq!(list.get(1).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server2");
        assert_eq!(list.get(2).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server0");
        assert!(list.move_index(2, 0));
        assert_eq!(list.get(0).unwrap().as_compound().unwrap().find_string("name").unwrap(), "server0");
        assert!(!list.move_index(0, 0));
        assert!(!list.move_index(5, 0));
    }

    #[test]
    fn remove_after_decode_with_root_not_zero() {
        // read_named inserts children before root, so root_index != 0; removing child 0 must not panic
        let nbt = make_servers_nbt(2);
        let bytes = crate::encode::write_named(&nbt);
        let mut decoded = crate::decode::read_named(&mut bytes.as_slice()).unwrap();
        // sanity: root_index is last slab index, not 0
        assert_ne!(decoded.root_index, 0);
        let mut root = decoded.as_compound_mut().unwrap();
        // remove a key that lives at slab idx 0 (first inserted string child) via compound remove
        // even if leaf, removing via ListRefMut::remove_index should not hit root guard
        let mut list = root.find_list_mut("servers", TAG_COMPOUND_ID).unwrap();
        assert!(list.remove_index(0));
        assert_eq!(list.len(), 1);
    }

    #[test]
    #[should_panic(expected = "Cannot remove root node")]
    fn remove_root_panics() {
        let mut nbt = NBT::new_named("root".into());
        let idx = nbt.root_index;
        nbt.remove_node(idx);
    }
}
