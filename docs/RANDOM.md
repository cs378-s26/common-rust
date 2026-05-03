# System Randomness

## Interfaces

### `Random` struct

An object of this type represents an instance of the pseudorandom generator.

```Rust
impl Random {
    pub fn new() -> Random;
    pub fn generate<T: Randomizable>(&mut self) -> T;
    pub fn inject(&mut self, data: &dyn Hashable);
    pub fn fork(&mut self) -> Random;
}
```

`new()` creates a brand-new generator, which will NOT be seeded with any entropy, and will therefore produce deterministic output. 
`generate()` produces a random value of the specified type, which must implement the `Randomizable` trait. 
`inject()` takes a reference to any value that implements the `Hashable` trait and injects it into the generator's state to increase its entropy. 
`fork()` creates a new generator whose state is derived from the current generator's state, which will produce different unpredictable output..

### `Seed`

This is a type alias for `[u8; 32]` (i.e., a slice of 32 bytes) that represents the randomness required to seed our pseudorandom generator. 

### `Hashable`

If you wish to inject a value of some type into the generator's state via `inject()`, you can implement this trait for that type by providing a single method `hash(&self) -> Seed`.

### `Randomizable`

If you wish to generate a random value of some type via `generate()`, you can implement this trait for that type by providing a single method `randomize(&mut self, rng: &mut Random)`.

To preserve typical semantics of pseudorandom number generation, the output of this method should appear to be uniformly distributed over the set of values of the type's range. For example, it would be a poor idea to implement `Randomizable` for `Vec<T: Randomizable>` by simply generating a random length and then filling the vector with that many copies of the same random value, since this would make it easy to predict the output after seeing just one value.

## Access Points

There is a public static variable `GLOBAL_RNG` in `random.rs`, as well as one inside every `Process` object to enforce safety against malicious processes learning information about the state of the generator.

Additionally, `dev/random` is a character device exposed through the filesystem to the user that fills buffers with random bytes upon reading, and upon writing injects bytes into the generator's state to increase its entropy. It currently uses, since we do not reliably run in or indeed even attached to a process state.

## Implementation

### Pseudorandom Number Generation

We use the `rand_chacha` crate for pseudorandom number generation, specificially the `ChaCha20Rng` stream cipher, which is widely believed to be cryptographically secure, in the sense that no known efficient adversarial algorithm can distinguish its output from true uniform randomness, even given past outputs.

### Entropy Sources

Pseudorandom generators require "seeding" with some form of true randomness (which need not be fully uniform, just sufficiently difficult to predict) to generate unpredictable output. We currently rely on reading the architecture's timestamp counter many times (which depend on synchronization and are therefore hopefully difficult to predict) during startup to seed the global generator, and "fork" this generator (i.e., use its output to seed a new generator) for each process.

Authored by Michael Jennings