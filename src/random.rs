use core::array::from_fn;

// "never, ever try to roll your own crypto" - david wu
use rand::{Rng, SeedableRng, prelude::StdRng};

pub struct Random(StdRng);
pub struct Seed([u8; 32]);

/// Combines two seeds together
/// # Safety
/// For every value of new, must be a bijection from values of old to returned values
fn combine(mut old: Seed, new: Seed) -> Seed {
    for i in 0..32 {
        old.0[i] ^= new.0[i]
    }
    old
}

// TODO make trait implementation easier with macros

pub trait Hashable {
    fn hash(&self) -> Seed;
}

//  TODO numeric types can be further special cases
impl Hashable for u64 {
    fn hash(&self) -> Seed {
        let mut buffer = [0; 32];
        for i in 0..core::mem::size_of::<u64>() {
            buffer[i] = (self >> (8 * i)) as u8
        }
        Seed(buffer)
    }
}

impl Hashable for u8 {
    fn hash(&self) -> Seed {
        let mut buffer = [0; 32];
        buffer[0] = *self;
        Seed(buffer)
    }
}

// TODO think about this after the interface changes
// impl<T: Hashable, const N: usize> Hashable for [T; N] {
//     fn hash(&self) -> Seed {
//         let mut buffer = [0; 32];
//         for i in 0..core::mem::size_of::<u64>() {
//             buffer[i] = (self >> 8 * i) as u8
//         }
//         Seed(buffer)
//     }
// }

// rust sucks :(
// impl<T: Iterator> Hashable for T where T::Item: Hashable {
//     fn hash(&self) -> Seed {
//         if let Some(next) = T::next(&mut self) {
//             combine(self.hash(), next.hash())
//         } else {
//             Seed([0; 32])
//         }
//     }
// }

pub trait Randomizable {
    fn generate(rng: &mut Random) -> Self;
}

impl Randomizable for u64 {
    fn generate(rng: &mut Random) -> Self {
        rng.0.next_u64()
    }
}

impl Randomizable for u32 {
    fn generate(rng: &mut Random) -> Self {
        rng.0.next_u32()
    }
}

impl<T: Randomizable, const N: usize> Randomizable for [T; N] {
    fn generate(rng: &mut Random) -> Self {
        from_fn(|_| rng.generate())
    }
}

impl Random {
    pub fn new() -> Random {
        Random(StdRng::from_seed([0; 32]))
    }
    fn fill(&mut self, buffer: &mut [u8]) {
        self.0.fill_bytes(buffer);
    }
    pub fn generate<T: Randomizable>(&mut self) -> T {
        T::generate(self)
    }
    pub fn inject(&mut self, data: &dyn Hashable) {
        let mut buffer = [0; 32];
        self.fill(&mut buffer);
        self.0 = StdRng::from_seed(combine(Seed(buffer), data.hash()).0);
    }
    pub fn fork(&mut self) -> Random {
        Random(self.0.fork())
    }
}

#[cfg(test)]
mod test {
    use crate::random::Random;

    #[test_case]
    fn test_rng() {
        // can't print anything for now, so just checking if it runs
        let mut rng = Random::new();
        rng.generate::<u64>();
        let mut buffer = [0; 42];
        rng.fill(&mut buffer);
        rng.inject(&mut buffer[0]);
    }
}