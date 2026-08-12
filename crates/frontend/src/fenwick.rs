// ponytail: inline FenwickTree<usize>. The standalone ftree crate used generic T with serde and ntest (519 lines).
// The frontend uses only FenwickTree<usize> for virtual list item sizes.
// This module keeps only the methods that the virtual list uses: new, from_iter, push, add_at, sub_at, and index_of_with_remainder.
// The module has about 80 lines and replaces 519 lines of crate code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenwickTree {
    inner: Vec<usize>,
}

impl FromIterator<usize> for FenwickTree {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = usize>,
    {
        let mut inner: Vec<usize> = iter.into_iter().collect();
        let n = inner.len();
        for i in 0..n {
            let parent = i | (i + 1);
            if parent < n {
                let child = inner[i];
                inner[parent] += child;
            }
        }
        Self { inner }
    }
}

impl FenwickTree {
    pub const fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn add_at(&mut self, index: usize, diff: usize) {
        let mut i = index;
        while let Some(v) = self.inner.get_mut(i) {
            *v += diff;
            i |= i + 1;
        }
    }

    pub fn sub_at(&mut self, index: usize, diff: usize) {
        let mut i = index;
        while let Some(v) = self.inner.get_mut(i) {
            *v -= diff;
            i |= i + 1;
        }
    }

    pub fn push(&mut self, value: usize) {
        let index = self.inner.len();
        self.inner.push(value);
        let lower_one_bits = (!index).trailing_zeros();
        for i in 0..lower_one_bits {
            let child = index & !(1 << i);
            let child_val = self.inner[child];
            self.inner[index] += child_val;
        }
    }

    // Used by virtual list to find which item contains `prefix_sum`.
    pub fn index_of_with_remainder(&self, mut prefix_sum: usize) -> (usize, usize) {
        let mut index = 0;
        let mut probe = most_significant_bit(self.inner.len()) * 2;
        while probe > 0 {
            let lsb = least_significant_bit(probe);
            let half_lsb = lsb / 2;
            let other_half_lsb = lsb - half_lsb;
            if let Some(value) = self.inner.get(probe - 1)
                && *value < prefix_sum
            {
                index = probe;
                prefix_sum -= *value;
                probe += half_lsb;
                if half_lsb > 0 {
                    continue;
                }
            }
            if lsb % 2 > 0 {
                break;
            }
            probe -= other_half_lsb;
        }
        (index, prefix_sum)
    }
}

const fn least_significant_bit(n: usize) -> usize {
    n & n.wrapping_neg()
}

const fn most_significant_bit(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        1 << (usize::BITS - 1 - n.leading_zeros())
    }
}

#[cfg(test)]
mod tests {
    use super::FenwickTree;

    #[test]
    fn test_from_iter() {
        let f = FenwickTree::from_iter([1, 6, 3, 9, 2]);
        assert_eq!(f.inner, vec![1, 7, 3, 19, 2]);
    }

    #[test]
    fn test_push_and_add() {
        let mut f = FenwickTree::from_iter([1, 6, 3]);
        f.push(9);
        // Cumulative sizes are [1, 7, 10, 19]; remainder is offset within the item.
        assert_eq!(f.index_of_with_remainder(10), (2, 3));
        f.add_at(0, 1);
        assert_eq!(f.inner[0], 2);
    }
}
