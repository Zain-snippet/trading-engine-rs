//! Initial workspace scaffold for the Rust trading engine.
//!
//! Trading functionality is intentionally not implemented yet. See `README.md`
//! for the staged migration philosophy and the current project status.

fn main() {
    println!("trading-engine: initial setup complete; no trading logic implemented yet.");
}

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_supports_cargo_test() {
        assert_eq!(2 + 2, 4);
    }
}
