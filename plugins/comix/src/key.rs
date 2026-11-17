//! Request-signing key material observed from the site's API client (spec section 4).
//!
//! The key id and the material change only on a deliberate key rotation; the server reports a
//! retired id as `key_retired`. The material is pinned by digest in the tests, not by the module
//! hash, which changes with every frontend deploy.

/// Key id the server expects as the prefix of every `_` token.
pub const KEY_ID: &str = "etu";

/// Two SHA-256 chaining states (bytes 0..32 inner, 32..64 outer), then the 1024-byte table that
/// decodes sealed responses.
pub static KEY_MATERIAL: [u8; 1088] = hex_bytes(concat!(
    "06bb7363060a516808903f7e6ffe9573cf352b0df82fe9d44ebd5b87b72556c66d5190d8e184e75d173cb5056edb2c8e932b1ac6deb6c5930d34b3995deffb09",
    "f2cc9a7592dd61e5a84a2100e8f0357ba7fa057c4ba2b38a0b5b85913c1747abd3cc7fd3c6e225e7ef3c2ff55cbc85b65ac1d48a8419c34a897e49e0eaf59a85",
    "8dfddc3fa7275ed03ed75ed7719b03bc8ef88d4805753b61005635ce1374171a3169df38822c565d6f5e74aac3aa7885b74fc903b3bdc9609307a50d450c396b",
    "5695c1e21da1a29e019f2d0f3f7b88d393bb5eec844ddd467d00ed48bbf2ca15b334f16bd79b547a56e53a3b24468cc88debf6a7f318b5ac4a5bc4c49c06dca8",
    "773a919f2e384ff95dd51e2313142dedefff291c433573149efe9cdec4f7e97d26d0dd1c1fcfed9ca7c548a12c570272ee6cf3e9e43f77dc7cea33a3444a8dae",
    "549a097475d4aa406e37d6c7ab291265e9356866b04fda05948c02f4e230b0fdcf65ea6b0537b7053035b5fb8f641f7d413951e8e624b5c8020e03df3d025056",
    "fbffe8441d8f5e48a75f7adc2e15c27e18938f995a02d8d551c6f0237037689bce8184abfd3699f33cf37b201b3be232bc185ad7fa91a464407e0226d88852f4",
    "0a09bbc7a65275248714a81bf8c7b725aff829bb3d49effb7e653828dc994ac6bf422931dfbb32b153e226e8f73906c516139ecd7ac14bcab42ec0901ccd0cdf",
    "206e0e34d324ea7b23d368f95c609a6bfb8efe16241712d469170cc41c3663a3476f9f28779100e0286aae1fdf64be68847424d564d51ee6b2373a3091c20724",
    "a19b4b56352d900aaef469598075db2202592a53220cd704fe1211968470cc0a4b5c5a5d0d4989614da71c58e136b66f06c38d25716abd88120790cb3213f92f",
    "50b5d85b7eba89195508a806cd08c450a7ca7566d07934c3a6155a4c6af25349271056cfa48004ad489401c76f8e89cfbefdefcd5d216d65acf9a92f9fda37fa",
    "0c5c129da1f2033d9e03236ef406a1385de289839148b1dfdbc9381dd8a2bcba275be3f431fa5fc25309d817f432d531c5e8317fffa197af48de51dda7c08cf3",
    "a7301d8f112d8541d3074211e91b750f27da3a57af6e54aee48885a72468d0fa5dc7059045c7c6a53249ae062e9846ad1063cd5c5b60f0d534ec5879dd267d92",
    "b0818f3344c07a6158a5873459185a33b35d4703e2e8b62beaab977aca7edaceb40310e96477bd914c495093e259794555d52e0fcf81ad425c44fc17dc8b4b71",
    "ba322ca535e12db2703caf22352c1eb61d476bda093b47c6c95e08c98ba9b2cc3f20c0ec9df555e2acdfc7dd0f41eadd7c728a9bb8c4e8438a9fdf923db0093d",
    "81fa3de4e3015871a091620b8162600c8d58424f800cb3531241e03020975650a7cf3700242d5007c33cd7b1c4954565790945c4335d3e7d26fd1f71c7e98f07",
    "6683a21d41344a393112b7a43ef7e1d8e3b686987c640f1bd9d2328e4b40e66d070dfb0f4b457b71a753a6c76c52aa768bd75be4a2ef82556b9c306d41d17e38",
));

/// Decodes a lowercase hex literal at compile time, so the material costs nothing at run time.
const fn hex_bytes<const N: usize>(hex: &str) -> [u8; N] {
    let hex = hex.as_bytes();
    assert!(hex.len() == 2 * N, "key material has the wrong length");
    let mut out = [0u8; N];
    let mut index = 0;
    while index < N {
        out[index] = (nibble(hex[2 * index]) << 4) | nibble(hex[2 * index + 1]);
        index += 1;
    }
    out
}

const fn nibble(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => panic!("key material is not lowercase hex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// SHA-256 of the 1088 material bytes. A changed key shows up here before any request does.
    const KEY_SHA256: &str = "14e44398f007c8d6557036779a50b7412430dee7d382ed172ecb42808c612fd8";

    #[test]
    fn material_matches_its_pinned_digest() {
        let digest = Sha256::digest(KEY_MATERIAL);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, KEY_SHA256);
        assert_eq!(KEY_ID, "etu");
    }
}
