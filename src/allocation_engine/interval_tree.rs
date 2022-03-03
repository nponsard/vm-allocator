// Copyright (C) 2022 Alibaba Cloud. All rights reserved.
// Copyright 2022 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2

use crate::{Error, Result};
use std::cmp::{max, min, Ordering};

/// Policy for resource allocation.
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum AllocPolicy {
    /// Allocate the first matched entry.
    FirstMatch,
    /// Allocate first matched entry from the end of the range.
    LastMatch,
    /// Allocate a memory slot starting with the specified address
    /// if it is available.
    ExactMatch(u64),
}

impl Default for AllocPolicy {
    fn default() -> Self {
        AllocPolicy::FirstMatch
    }
}

/// Struct to describe resource allocation constraints.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Constraint {
    /// Size to allocate.
    pub size: u64,
    /// Range where the allocated resource will be placed.
    pub desired_range: Range,
    /// Alignment for the allocated resource.
    pub align: u64,
    /// Resource allocation policy.
    pub policy: AllocPolicy,
}

/// A closed interval range [start, end] used to describe a
/// memory slot that will be assigned to a device by the VMM.
/// This structure represents the key of the Node object in
/// the interval tree implementation.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Hash, Ord, Debug)]
pub struct Range {
    /// Lower boundary of the interval.
    start: u64,
    /// Upper boundary of the interval.
    end: u64,
}

#[allow(clippy::len_without_is_empty)]
impl Range {
    /// Create a new Range object.
    pub fn new(start: u64, end: u64) -> Result<Self> {
        if start > end {
            return Err(Error::InvalidRange(start, end));
        }
        Ok(Range { start, end })
    }

    /// Get length of the range.
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Check whether two Range objects overlap with each other.
    pub fn overlaps(&self, other: &Range) -> bool {
        max(self.start, other.start) <= min(self.end, other.end)
    }

    /// Check whether the key is fully covered.
    pub fn contains(&self, other: &Range) -> bool {
        self.start <= other.start && self.end >= other.end
    }

    /// Get the lower boundary of the range.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// Get the upper boundary of the range.
    pub fn end(&self) -> u64 {
        self.end
    }
}

/// Returns the first multiple of `alignment` that is lower or equal to the
/// specified address. This method works only for alignment values that are a
/// power of two.
///
/// # Examples
/// ```rust
/// extern crate vm_allocator;
/// use vm_allocator::allocation_engine::{align_down, Range};
/// use vm_allocator::Error;
/// use vm_allocator::Result;
///
/// fn intervals_align_down() -> Result<u64> {
///     let address = 5;
///     if let Ok(res) = align_down(address, 2) {
///         if res == 4 {
///             return Ok(res);
///         }
///         return Err(Error::Overflow);
///     }
///     Err(Error::Overflow)
/// }
///
/// # intervals_align_down().unwrap();
/// ```
pub fn align_down(address: u64, alignment: u64) -> Result<u64> {
    if !alignment.is_power_of_two() {
        return Err(Error::InvalidAlignment);
    }
    Ok(address & !(alignment - 1))
}

/// Returns the first multiple of `alignment` that is greater or equal to the
/// specified address. This method works only for alignment values that are a
/// power of two.
///
/// # Examples
/// ```rust
/// extern crate vm_allocator;
/// use vm_allocator::allocation_engine::{align_up, Range};
/// use vm_allocator::Error;
/// use vm_allocator::Result;
///
/// fn intervals_align_up() -> Result<u64> {
///     let address = 3;
///     if let Ok(res) = align_up(address, 2) {
///         if res == 4 {
///             return Ok(res);
///         }
///         return Err(Error::Overflow);
///     }
///     Err(Error::Overflow)
/// }
///
/// # intervals_align_up().unwrap();
/// ```
pub fn align_up(address: u64, alignment: u64) -> Result<u64> {
    if alignment == 0 {
        return Err(Error::InvalidAlignment);
    }
    if let Some(intermediary_address) = address.checked_add(alignment - 1) {
        return align_down(intermediary_address, alignment);
    }
    Err(Error::Overflow)
}

/// Node state for interval tree nodes.
///
/// Valid state transition:
/// - None -> Free: IntervalTree::insert()
/// - Free -> Allocated: IntervalTree::allocate()
/// - Allocated -> Free: IntervalTree::free()
/// - * -> None: IntervalTree::delete()
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub enum NodeState {
    /// Node is free.
    Free,
    /// Node is allocated.
    Allocated,
}

impl NodeState {
    #[allow(dead_code)]
    pub(crate) fn is_free(&self) -> bool {
        *self == NodeState::Free
    }
}

/// Internal tree node to implement interval tree.
#[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct InnerNode {
    /// Interval handled by this node.
    key: Range,
    /// NodeState, can be Free or Allocated.
    node_state: NodeState,
    /// Optional left child of current node.
    left: Option<Box<InnerNode>>,
    /// Optional right child of current node.
    right: Option<Box<InnerNode>>,
    /// Cached height of the node.
    height: u64,
}

impl InnerNode {
    /// Creates a new InnerNode object.
    #[allow(dead_code)]
    fn new(key: Range, node_state: NodeState) -> Self {
        InnerNode {
            key,
            node_state,
            left: None,
            right: None,
            height: 1,
        }
    }

    /// Returns a readonly reference to the node associated with the `key` or
    /// None if the searched key does not exist in the tree.
    #[allow(dead_code)]
    fn search(&self, key: &Range) -> Option<&InnerNode> {
        match self.key.cmp(key) {
            Ordering::Equal => Some(self),
            Ordering::Less => self.right.as_ref().and_then(|node| node.search(key)),
            Ordering::Greater => self.left.as_ref().and_then(|node| node.search(key)),
        }
    }

    /// Returns a readonly reference to the node associated with the `key` or
    /// None if there is no Node representing an interval that covers the
    /// searched key. For a key [a, b], this method will return a node with
    /// a key [c, d] such that c <= a and b <= d.
    #[allow(dead_code)]
    fn search_superset(&self, key: &Range) -> Option<&InnerNode> {
        if self.key.contains(key) {
            Some(self)
        } else if key.end < self.key.start {
            self.left
                .as_ref()
                .and_then(|node| node.search_superset(key))
        } else {
            self.right
                .as_ref()
                .and_then(|node| node.search_superset(key))
        }
    }

    /// Rotates the tree such that height difference between subtrees
    /// is not greater than abs(1).
    #[allow(dead_code)]
    fn rotate(self: Box<Self>) -> Box<Self> {
        let l = height(&self.left);
        let r = height(&self.right);

        match (l as i64) - (r as i64) {
            1 | 0 | -1 => self,
            // Safe to unwrap as rotate_left_successor always returns Some when
            // the current node has a left child and we just checked that it
            // has at least one child otherwise this difference would not be two.
            2 => self.rotate_left_successor().unwrap(),
            // Safe to unwrap as rotate_right_successor always returns Some when
            // the current node has a right child and we just checked that it
            // has at least one child otherwise this difference would not be
            // minus two.
            -2 => self.rotate_right_successor().unwrap(),
            _ => unreachable!(),
        }
    }

    /// Performs a single left rotation on this node.
    #[allow(dead_code)]
    fn rotate_left(mut self: Box<Self>) -> Option<Box<Self>> {
        if let Some(mut new_root) = self.right.take() {
            self.right = new_root.left.take();
            self.update_cached_height();
            new_root.left = Some(self);
            new_root.update_cached_height();
            return Some(new_root);
        }
        None
    }

    /// Performs a single right rotation on this node.
    #[allow(dead_code)]
    fn rotate_right(mut self: Box<Self>) -> Option<Box<Self>> {
        if let Some(mut new_root) = self.left.take() {
            self.left = new_root.right.take();
            self.update_cached_height();
            new_root.right = Some(self);
            new_root.update_cached_height();
            return Some(new_root);
        }
        None
    }

    /// Performs a rotation when the left successor is too high.
    #[allow(dead_code)]
    fn rotate_left_successor(mut self: Box<Self>) -> Option<Box<Self>> {
        if let Some(left) = self.left.take() {
            if height(&left.left) < height(&left.right) {
                self.left = left.rotate_left();
                self.update_cached_height();
            } else {
                self.left = Some(left);
            }
            return self.rotate_right();
        }
        None
    }

    /// Performs a rotation when the right successor is too high.
    #[allow(dead_code)]
    fn rotate_right_successor(mut self: Box<Self>) -> Option<Box<Self>> {
        if let Some(right) = self.right.take() {
            if height(&right.left) > height(&right.right) {
                self.right = right.rotate_right();
                self.update_cached_height();
            } else {
                self.right = Some(right);
            }
            return self.rotate_left();
        }
        None
    }

    /// Deletes the entry point of this tree structure.
    #[allow(dead_code)]
    fn delete_root(mut self) -> Option<Box<Self>> {
        match (self.left.take(), self.right.take()) {
            (None, None) => None,
            (Some(l), None) => Some(l),
            (None, Some(r)) => Some(r),
            (Some(l), Some(r)) => Some(Self::combine_subtrees(l, r)),
        }
    }

    /// Finds the minimal key below the tree and returns a new optional tree
    /// where the minimal value has been removed and the (optional) minimal node
    /// as tuple (min_node, remaining).
    #[allow(dead_code)]
    fn get_new_root(mut self: Box<Self>) -> (Box<Self>, Option<Box<Self>>) {
        match self.left.take() {
            None => {
                let remaining = self.right.take();
                (self, remaining)
            }
            Some(left) => {
                let (min_node, left) = left.get_new_root();
                self.left = left;
                self.update_cached_height();
                (min_node, Some(self.rotate()))
            }
        }
    }

    /// Creates a single tree from the subtrees resulted from deleting the root
    /// node.
    #[allow(dead_code)]
    fn combine_subtrees(l: Box<Self>, r: Box<Self>) -> Box<Self> {
        let (mut new_root, remaining) = r.get_new_root();
        new_root.left = Some(l);
        new_root.right = remaining;
        new_root.update_cached_height();
        new_root.rotate()
    }

    /// Updates cached information of the node.
    #[allow(dead_code)]
    fn update_cached_height(&mut self) {
        self.height = max(height(&self.left), height(&self.right)) + 1;
    }

    /// Insert a new node in the subtree. After the node is inserted the
    /// tree will be balanced. The node_state parameter is needed because in
    /// the AddressAllocator allocation logic we will need to insert both free
    /// and allocated nodes.
    #[allow(dead_code)]
    pub(crate) fn insert(
        mut self: Box<Self>,
        key: Range,
        node_state: NodeState,
    ) -> Result<Box<Self>> {
        // The InnerNode structure has 48 a length of 48 bytes. With other nested
        // calls that are made during the insertion process the size occupied
        // on the stack by just one insert call is around 122 bytes. Considering
        // that the default stack size on Linux is 8K we could make around 73
        // calls to insert method before confronting with an stack overflow. To
        // be cautious we will use 50 as the maximum height of the tree. A
        // maximum height of 50 will result in the possibility to allocate
        // (2^50 - 1) memory slots. Considering the imposed maximum height the
        // recursion is safe to use.
        if (self.height + 1) > 50 {
            return Err(Error::Overflow);
        }
        if self.key.overlaps(&key) {
            return Err(Error::Overlap(key, self.key));
        }
        match self.key.cmp(&key) {
            // It is not possible for a range to be equal with an existing node
            // as the overlaps method will also catch this case and return the
            // corresponding error code.
            Ordering::Equal => unreachable!(),
            Ordering::Less => match self.right {
                None => self.right = Some(Box::new(InnerNode::new(key, node_state))),
                Some(right) => {
                    self.right = Some(right.insert(key, node_state)?);
                }
            },
            Ordering::Greater => match self.left {
                None => self.left = Some(Box::new(InnerNode::new(key, node_state))),
                Some(left) => {
                    self.left = Some(left.insert(key, node_state)?);
                }
            },
        }
        self.update_cached_height();
        Ok(self.rotate())
    }
}

/// Compute height of the optional sub-tree.
#[allow(dead_code)]
fn height(node: &Option<Box<InnerNode>>) -> u64 {
    node.as_ref().map_or(0, |n| n.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_range() {
        assert_eq!(Range::new(2, 1).unwrap_err(), Error::InvalidRange(2, 1));
    }

    #[test]
    fn test_range_overlaps() {
        let range_a = Range::new(1u64, 4u64).unwrap();
        let range_b = Range::new(4u64, 6u64).unwrap();
        let range_c = Range::new(2u64, 3u64).unwrap();
        let range_d = Range::new(4u64, 4u64).unwrap();
        let range_e = Range::new(5u64, 6u64).unwrap();

        assert!(range_a.overlaps(&range_b));
        assert!(range_b.overlaps(&range_a));
        assert!(range_a.overlaps(&range_c));
        assert!(range_c.overlaps(&range_a));
        assert!(range_a.overlaps(&range_d));
        assert!(range_d.overlaps(&range_a));
        assert!(!range_a.overlaps(&range_e));
        assert!(!range_e.overlaps(&range_a));

        assert_eq!(range_a.len(), 4);
        assert_eq!(range_d.len(), 1);
    }

    #[test]
    fn test_range_contain() {
        let range_a = Range::new(2u64, 6u64).unwrap();
        assert!(range_a.contains(&Range::new(2u64, 3u64).unwrap()));
        assert!(range_a.contains(&Range::new(3u64, 4u64).unwrap()));
        assert!(range_a.contains(&Range::new(5u64, 5u64).unwrap()));
        assert!(range_a.contains(&Range::new(5u64, 6u64).unwrap()));
        assert!(range_a.contains(&Range::new(6u64, 6u64).unwrap()));
        assert!(!range_a.contains(&Range::new(1u64, 1u64).unwrap()));
        assert!(!range_a.contains(&Range::new(1u64, 2u64).unwrap()));
        assert!(!range_a.contains(&Range::new(1u64, 3u64).unwrap()));
        assert!(!range_a.contains(&Range::new(1u64, 7u64).unwrap()));
        assert!(!range_a.contains(&Range::new(7u64, 8u64).unwrap()));
        assert!(!range_a.contains(&Range::new(6u64, 7u64).unwrap()));
        assert!(!range_a.contains(&Range::new(7u64, 8u64).unwrap()));
    }

    #[test]
    fn test_range_align_up() {
        assert_eq!(align_up(2, 0).unwrap_err(), Error::InvalidAlignment);
        assert_eq!(align_up(2, 1).unwrap(), 2);
        assert_eq!(align_up(2, 2).unwrap(), 2);
        assert_eq!(align_up(2, 4).unwrap(), 4);
        assert_eq!(align_up(2, 3).unwrap_err(), Error::InvalidAlignment);

        assert_eq!(
            align_up(0xFFFF_FFFF_FFFF_FFFDu64, 2).unwrap(),
            0xFFFF_FFFF_FFFF_FFFEu64
        );
        assert_eq!(
            align_up(0xFFFF_FFFF_FFFF_FFFDu64, 4).unwrap_err(),
            Error::Overflow
        );
    }

    #[test]
    fn test_range_ord() {
        let range_a = Range::new(1u64, 4u64).unwrap();
        let range_b = Range::new(1u64, 4u64).unwrap();
        let range_c = Range::new(1u64, 3u64).unwrap();
        let range_d = Range::new(1u64, 5u64).unwrap();
        let range_e = Range::new(2u64, 2u64).unwrap();

        assert_eq!(range_a, range_b);
        assert_eq!(range_b, range_a);
        assert!(range_a > range_c);
        assert!(range_c < range_a);
        assert!(range_a < range_d);
        assert!(range_d > range_a);
        assert!(range_a < range_e);
        assert!(range_e > range_a);
    }

    #[test]
    fn test_getters() {
        let range = Range::new(3, 5).unwrap();
        assert_eq!(range.start(), 3);
        assert_eq!(range.end(), 5);
    }

    #[test]
    fn test_is_free() {
        let mut ns = NodeState::Allocated;
        assert!(!ns.is_free());
        ns = NodeState::Free;
        assert!(ns.is_free());
    }

    #[test]
    fn test_search() {
        let mut tree = Box::new(InnerNode::new(
            Range::new(0x100, 0x110).unwrap(),
            NodeState::Allocated,
        ));
        let left_child = InnerNode::new(Range::new(0x90, 0x99).unwrap(), NodeState::Free);

        tree = tree.insert(left_child.key, left_child.node_state).unwrap();
        tree = tree
            .insert(Range::new(0x200, 0x2FF).unwrap(), NodeState::Free)
            .unwrap();

        assert_eq!(
            tree.search(&Range::new(0x90, 0x99).unwrap()),
            Some(&left_child)
        );
        assert_eq!(tree.search(&Range::new(0x200, 0x250).unwrap()), None);
        assert_eq!(tree.search(&Range::new(0x111, 0x1fe).unwrap()), None);
    }

    #[test]
    fn test_search_superset() {
        let mut tree = Box::new(InnerNode::new(
            Range::new(0x100, 0x110).unwrap(),
            NodeState::Allocated,
        ));
        let right_child = InnerNode::new(Range::new(0x200, 0x2FF).unwrap(), NodeState::Free);
        let left_child = InnerNode::new(Range::new(0x90, 0x9F).unwrap(), NodeState::Free);

        tree = tree.insert(left_child.key, left_child.node_state).unwrap();
        tree = tree
            .insert(right_child.key, right_child.node_state)
            .unwrap();

        assert_eq!(
            tree.search_superset(&Range::new(0x100, 0x100).unwrap()),
            Some(&(*tree))
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x90, 0x95).unwrap()),
            Some(&left_child)
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x200, 0x201).unwrap()),
            Some(&right_child)
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x200, 0x2FF).unwrap()),
            Some(&right_child)
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x210, 0x210).unwrap()),
            Some(&right_child)
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x2FF, 0x2FF).unwrap()),
            Some(&right_child)
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x2FF, 0x300).unwrap()),
            None
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x300, 0x300).unwrap()),
            None
        );
        assert_eq!(
            tree.search_superset(&Range::new(0x1ff, 0x300).unwrap()),
            None
        );
    }

    fn is_balanced(tree: Option<Box<InnerNode>>) -> bool {
        if tree.is_none() {
            return true;
        }
        let left_height = height(&tree.as_ref().unwrap().left.clone());
        let right_height = height(&tree.as_ref().unwrap().right.clone());
        if (left_height as i64 - right_height as i64).abs() <= 1
            && is_balanced(tree.as_ref().unwrap().left.clone())
            && is_balanced(tree.as_ref().unwrap().right.clone())
        {
            return true;
        }
        false
    }

    #[test]
    fn test_tree_insert_balanced() {
        let mut tree = Box::new(InnerNode::new(
            Range::new(0x300, 0x310).unwrap(),
            NodeState::Allocated,
        ));
        tree = tree
            .insert(Range::new(0x100, 0x110).unwrap(), NodeState::Free)
            .unwrap();
        tree = tree
            .insert(Range::new(0x90, 0x9F).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        tree = tree
            .insert(Range::new(0x311, 0x313).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        tree = tree
            .insert(Range::new(0x314, 0x316).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        tree = tree
            .insert(Range::new(0x317, 0x319).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        tree = tree
            .insert(Range::new(0x321, 0x323).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree)));
    }

    #[test]
    fn test_tree_insert_intersect_negative() {
        let mut tree = Box::new(InnerNode::new(
            Range::new(0x100, 0x200).unwrap(),
            NodeState::Allocated,
        ));
        tree = tree
            .insert(Range::new(0x201, 0x2FF).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        let res = tree
            .clone()
            .insert(Range::new(0x201, 0x2FE).unwrap(), NodeState::Free);
        assert_eq!(
            res.unwrap_err(),
            Error::Overlap(
                Range::new(0x201, 0x2FE).unwrap(),
                Range::new(0x201, 0x2FF).unwrap()
            )
        );
        tree = tree
            .insert(Range::new(0x90, 0x9F).unwrap(), NodeState::Free)
            .unwrap();
        assert!(is_balanced(Some(tree.clone())));
        let res = tree.insert(Range::new(0x90, 0x9E).unwrap(), NodeState::Free);
        assert_eq!(
            res.unwrap_err(),
            Error::Overlap(
                Range::new(0x90, 0x9E).unwrap(),
                Range::new(0x90, 0x9F).unwrap()
            )
        );
    }

    #[test]
    fn test_tree_insert_duplicate_negative() {
        let range = Range::new(0x100, 0x200).unwrap();
        let tree = Box::new(InnerNode::new(range, NodeState::Allocated));
        let res = tree.insert(range, NodeState::Free);
        assert_eq!(res.unwrap_err(), Error::Overlap(range, range));
    }

    #[test]
    fn test_tree_stack_overflow_negative() {
        let mut inner_node =
            InnerNode::new(Range::new(0x100, 0x200).unwrap(), NodeState::Allocated);
        inner_node.height = 50;
        let tree = Box::new(inner_node);
        let res = tree.insert(Range::new(0x100, 0x200).unwrap(), NodeState::Free);
        assert_eq!(res.unwrap_err(), Error::Overflow);
    }
}
