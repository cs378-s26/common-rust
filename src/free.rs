use alloc::rc::Rc;
use intrusive_collections::{Bound::Included, KeyAdapter, RBTree, RBTreeLink, intrusive_adapter};

struct Free {
    base: usize,
    length: usize,
    base_link: RBTreeLink,
    length_link: RBTreeLink,
}

intrusive_adapter!(FreeByBaseAdapter = Rc<Free>: Free { base_link => RBTreeLink });
impl<'a> KeyAdapter<'a> for FreeByBaseAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a Free) -> usize {
        x.base
    }
}

intrusive_adapter!(FreeByLengthAdapter = Rc<Free>: Free { length_link => RBTreeLink });
impl<'a> KeyAdapter<'a> for FreeByLengthAdapter {
    type Key = usize;
    fn get_key(&self, x: &'a Free) -> usize {
        x.length
    }
}

pub struct FreeSet {
    base_set: RBTree<FreeByBaseAdapter>,
    length_set: RBTree<FreeByLengthAdapter>,
}

impl FreeSet {}

impl FreeSet {
    pub fn new() -> Self {
        Self {
            base_set: RBTree::new(FreeByBaseAdapter::new()),
            length_set: RBTree::new(FreeByLengthAdapter::new()),
        }
    }
    // this could definitely be optimized (especially if we use unsafe)
    pub fn add_range(&mut self, base: usize, length: usize) -> Result<(), &'static str> {
        if length == 0 {
            return Err("trying to add an empty range");
        }
        let mut coalesce_left = None;
        if let Some(free) = self.base_set.upper_bound(Included(&base)).get() {
            if free.base + free.length > base {
                return Err("added range conflicts with upper part of existing range");
            }
            if free.base + free.length == base {
                coalesce_left = Some((free.base, free.length));
            }
        }
        let mut coalesce_right = None;
        if let Some(free) = self.base_set.lower_bound(Included(&base)).get() {
            if base + length > free.base {
                return Err("added range conflicts with lower part of existing range");
            }
            if base + length == free.base {
                coalesce_right = Some((free.base, free.length));
            }
        }
        let mut coalesced_base = base;
        let mut coalesced_length = length;
        if let Some((free_base, free_length)) = coalesce_left {
            coalesced_base = free_base;
            coalesced_length += free_length;
            self.remove_from_sets(free_base);
        }
        if let Some((free_base, free_length)) = coalesce_right {
            coalesced_length += free_length;
            self.remove_from_sets(free_base);
        }
        self.add_to_sets(coalesced_base, coalesced_length);
        Ok(())
    }
    fn add_to_sets(&mut self, base: usize, length: usize) {
        let free = Rc::new(Free {
            base,
            length,
            base_link: RBTreeLink::new(),
            length_link: RBTreeLink::new(),
        });
        self.base_set.insert(free.clone());
        self.length_set.insert(free);
    }
    fn remove_from_sets(&mut self, base: usize) {
        let mut base_cursor = self.base_set.find_mut(&base);
        unsafe {
            self.length_set
                .cursor_mut_from_ptr(base_cursor.get().unwrap())
                .remove()
        };
        base_cursor.remove();
    }
    pub fn remove_range_by_base(&mut self, base: usize, length: usize) -> Result<(), &'static str> {
        if let Some(free) = self.base_set.upper_bound(Included(&base)).get() {
            if free.base + free.length < base + length {
                Err("range can't be cleanly removed")
            } else {
                self.remove_range_by_base_unchecked(base, length, free.base, free.length)
            }
        } else {
            Err("there is no free range <= base")
        }
    }
    fn remove_range_by_base_unchecked(
        &mut self,
        cut_base: usize,
        cut_length: usize,
        whole_base: usize,
        whole_length: usize,
    ) -> Result<(), &'static str> {
        self.remove_from_sets(whole_base);
        if whole_base < cut_base {
            self.add_to_sets(whole_base, cut_base - whole_base);
        }
        if cut_base + cut_length < whole_length + whole_base {
            self.add_to_sets(
                cut_base + cut_length,
                (whole_length + whole_base) - (cut_base + cut_length),
            );
        }
        Ok(())
    }
    pub fn remove_range_by_length(&mut self, length: usize) -> Result<usize, &'static str> {
        if let Some(free) = self.length_set.lower_bound(Included(&length)).get() {
            let whole_base = free.base;
            let whole_length = free.length;
            match self.remove_range_by_base_unchecked(whole_base, length, whole_base, whole_length)
            {
                Ok(()) => Ok(whole_base),
                Err(e) => Err(e),
            }
        } else {
            Err("there is no free range big enough")
        }
    }
}

impl Clone for FreeSet {
    fn clone(&self) -> Self {
        let mut new_base_set = RBTree::new(FreeByBaseAdapter::new());
        let mut new_length_set = RBTree::new(FreeByLengthAdapter::new());
        let mut cursor = self.base_set.cursor();
        while cursor.get().is_some() {
            let free = cursor.get().unwrap();
            let free = Rc::new(Free {
                base: free.base,
                length: free.length,
                base_link: RBTreeLink::new(),
                length_link: RBTreeLink::new(),
            });
            new_base_set.insert(free.clone());
            new_length_set.insert(free.clone());
            cursor.move_next();
        }
        Self {
            base_set: new_base_set,
            length_set: new_length_set,
        }
    }
}

// TODO: verify if i should actually do this

unsafe impl Send for FreeSet {}
unsafe impl Sync for FreeSet {}

#[cfg(test)]
mod test {
    use crate::free::FreeSet;

    #[test_case]
    fn test_coalesce() {
        let mut set = FreeSet::new();
        assert!(set.add_range(0, 5).is_ok());
        assert!(set.add_range(10, 5).is_ok());
        assert!(set.add_range(5, 5).is_ok());
        assert!(set.remove_range_by_base(0, 15).is_ok());
    }

    #[test_case]
    fn test_best_fit() {
        let mut set = FreeSet::new();
        assert!(set.add_range(0, 1).is_ok());
        assert!(set.add_range(10, 2).is_ok());
        assert!(set.add_range(20, 3).is_ok());

        assert!(set.remove_range_by_length(2).is_ok());
        assert!(set.remove_range_by_length(2).is_ok());
        assert!(set.remove_range_by_length(3).is_err());
        assert!(set.remove_range_by_length(1).is_ok());
    }
}
