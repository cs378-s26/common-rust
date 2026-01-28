use std::{
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    ops::Range,
    rc::Rc,
};

#[derive(Debug)]
pub struct IntervalMap<K: Ord + Debug + Copy, V> {
    map: BTreeMap<K, (K, V)>,
}

impl<K: Ord + Debug + Copy, V> IntervalMap<K, V> {
    pub fn new() -> Self {
        IntervalMap {
            map: BTreeMap::new(),
        }
    }

    pub fn get(&self, k: &K) -> Option<(Range<&K>, &V)> {
        if let Some((start, (end, value))) = self.map.range(..=k).next_back()
            && k >= start
            && k < end
        {
            return Some((start..end, value));
        }
        None
    }

    pub fn insert(&mut self, range: &Range<K>, value: V) -> bool {
        let start = range.start;
        let end = range.end;

        for (s, (e, _)) in self.map.range(start..=end) {
            if *s < end && *e > start {
                return false;
            }
        }

        self.map.insert(start, (end, value));
        true
    }

    pub fn remove(&mut self, key: K) -> Option<V> {
        if let Some((start, (end, _))) = self.map.range(..=key).next_back()
            && key >= *start
            && key < *end
        {
            let start = *start;
            return self.map.remove(&start).map(|(_, value)| value);
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = (Range<&K>, &V)> {
        self.map.iter().map(|(k, v)| (k..&v.0, &v.1))
    }
}

pub struct InternStringTable {
    entries: Vec<Rc<String>>,
    interned: HashMap<Rc<String>, usize>,
}

impl InternStringTable {
    pub fn new() -> InternStringTable {
        InternStringTable {
            entries: Vec::new(),
            interned: HashMap::new(),
        }
    }

    pub fn intern(&mut self, str: &String) -> usize {
        if let Some(res) = self.interned.get(str) {
            *res
        } else {
            let str_rc = Rc::new(str.clone());
            let index = self.entries.len();
            self.entries.push(str_rc.clone());
            self.interned.insert(str_rc, index);
            index
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) -> impl Fn(usize) -> usize + use<> {
        let mut buf = Vec::new();
        let mut str_offset_tab = Vec::new();

        for ent in &self.entries {
            str_offset_tab.push(buf.len());
            buf.extend_from_slice(ent.as_bytes());
            buf.push(0);
        }

        out.extend_from_slice(&buf.len().to_le_bytes());
        out.extend_from_slice(&buf);

        move |index| {
            if index == usize::MAX {
                usize::MAX
            } else {
                str_offset_tab[index]
            }
        }
    }

    pub fn lazy_intern_table(&mut self, table: &[String]) -> impl FnMut(usize) -> usize {
        let mut cache: Vec<_> = vec![usize::MAX; table.len()];

        move |f| {
            if f == usize::MAX {
                usize::MAX
            } else {
                if cache[f] == usize::MAX {
                    cache[f] = self.intern(&table[f]);
                }
                cache[f]
            }
        }
    }
}
