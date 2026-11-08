//! **A cache with a budget**: the least recently used entries go once what
//! is kept costs more than it may. Formulas cost one each; pictures cost
//! their bytes.

use std::collections::HashMap;
use std::hash::Hash;

pub(crate) struct Lru<K, V> {
    map: HashMap<K, (V, u64, usize)>,
    tick: u64,
    cost: usize,
    budget: usize,
}

impl<K: Hash + Eq + Clone, V: Clone> Lru<K, V> {
    pub fn new(budget: usize) -> Lru<K, V> {
        Lru { map: HashMap::new(), tick: 0, cost: 0, budget }
    }

    /// The value kept for `key`, now the most recently used.
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|e| {
            e.1 = tick;
            e.0.clone()
        })
    }

    /// Keep `value` at `cost`, letting the oldest go to stay in budget. The
    /// newest entry is always kept, however much it costs.
    pub fn put(&mut self, key: K, value: V, cost: usize) {
        self.tick += 1;
        if let Some(old) = self.map.insert(key.clone(), (value, self.tick, cost)) {
            self.cost -= old.2;
        }
        self.cost += cost;
        while self.cost > self.budget && self.map.len() > 1 {
            let oldest = self.map.iter().filter(|(k, _)| **k != key).min_by_key(|(_, e)| e.1).map(|(k, _)| k.clone());
            match oldest.and_then(|k| self.map.remove(&k)) {
                Some(gone) => self.cost -= gone.2,
                None => break,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// What is kept costs this much.
    pub fn cost(&self) -> usize {
        self.cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_goes_first_and_a_use_makes_it_new() {
        let mut c = Lru::new(3);
        for k in 0..3 {
            c.put(k, k * 10, 1);
        }
        assert_eq!(c.get(&0), Some(0)); // 0 is now the newest
        c.put(3, 30, 1);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get(&1), None, "1 was the oldest");
        assert_eq!(c.get(&0), Some(0));
        // One too big for the budget is still kept, alone.
        c.put(9, 90, 10);
        assert_eq!((c.len(), c.get(&9)), (1, Some(90)));
        assert_eq!(c.cost(), 10);
    }
}
