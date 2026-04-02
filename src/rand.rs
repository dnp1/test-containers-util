use nanorand::{Rng, WyRand};

pub fn rand_str(len: usize) -> String {
    let mut rng = WyRand::new();

    // The alphabet we want to pull from
    let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                    abcdefghijklmnopqrstuvwxyz\
                    0123456789";

    // Generate 8 random indices and map them to the charset
    (0..len)
        .map(|_| {
            let idx = rng.generate_range(0..charset.len());
            charset[idx] as char
        })
        .collect()
}
