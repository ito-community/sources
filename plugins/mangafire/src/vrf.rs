// Request signing for MangaFire's /api endpoints (`vrf` query parameter).
// Algorithm: three substitution stages over the canonical request string.
// Reference: https://github.com/Aidoku-Community/sources (multi.mangafire/src/vrf.rs),
// originally derived from disk-iq8/extensions-source VrfSigner.kt.
use base64::{Engine, engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}};

const TABLE_1: &str = "yINlmUNho8VYJT+ibTIP+9ESiULpVEtMOoD6U6lRE0R/xwXo/Xp9NrUgC4cw/Lmo33vUyjUE40kUoEWIr/fxfNNcq2s79ShQ5NhNrFnJ4hXPwOu/SuXzIbuTQKGFvfm08E9jvCfqAtoDqvQq3dVWPQFmJjgvkISBeXY3BgANR+yVnjGbcxZ47d6kLNfZPIayTq3/YGySb1KuVZodWp/WGNAO5pfMcpaK53Hhs0allBszaMaxuouOwdxbwgxIw6YunSsXjI05Yi0j9j4eHKfSXR8Ifo/Od+8iamRfCXTyvm7NGRGYdcQ0ywcK/u6RXhrbcCm4t2eCtrDgQVecJGkQ+A==";
const KEY_1: &str = "0Ec58JOY3uBzJK9m3zqIOpdlF7UFiax9DmA=";
const TABLE_2: &str = "IUFltCxD3Oc2cwCgkJffthaOg9cgPUb0LgW6H/VtfcF0kc5F25t+aWj6JH9VOhOaY0rAFdUxlDnl5BLNvwEJvQtP5qcw7vdb/K+chnbwnspSHT8mz5lqwz41TezG0hkO06FTjJZhsyNuFLDpD2ZZxQj/QIRcF90zpmQ7Byu483WsQqUE0C342HL+JXngRB6fRzxRyVTaKu83h7UYTJ0QMt6ixFh6S3F8gqkKwrGTL3jHNBsD45UnifK8+RGtishQV2K3rujLKEkiZxpr2dYcudFW4oFsDKhad3CLBvuyTqsCo4B7mL5IKQ1vXo/MOOvq1I1d8ar9X6Ttu5KF4fZgiA==";
const KEY_2: &str = "AAdjb1iPY8CiDmq9H34tKTBF8a3oDQ==";
const TABLE_3: &str = "NQHlu1/wVO5EmkwQymF810qqY2xG1k2obcas4Z9mCsPEIFl9pRIjFxbJ7ybMHbBckT5Ton85E0FOeHezbh/mjlEYpmpnlXOS8dgrqeq2KfxImTh1YK9y0PeMNhzA1OQzSY9brYOJq/l2QnE/hwOeZIhPixVSKIUlDb5vLcH6RWKxkIEMuP0bDwIqQ71AJJaEaMJL7A6YtyIwoRT+L5v4aZzodN/0+3nOGsfblFjgxSfPzVDjNFeNl5P26+kEC/8AHgdrpAbt3hHz3HrRN1Y6e+JHgF7ncFWnoF0y3THL1S71WgWGCa6KtSzTCCG58n68nTyj2T3Sshk7utqCtMi/ZQ==";
const KEY_3: &str = "DELOJgPsVaCcblDtTGMdHzM=";

fn encrypt_stage(data: &[u8], table: &[u8], key: &[u8], iv: u8) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut previous = iv;
    for (index, byte) in data.iter().enumerate() {
        previous = table[(byte ^ key[index % key.len()] ^ previous) as usize];
        output.push(previous);
    }
    output
}

/// Signs a canonical request string: path plus sorted, indexed query params (no `vrf`).
pub fn sign(canonical: &str) -> String {
    let stages = [
        (TABLE_1, KEY_1, 0x5A),
        (TABLE_2, KEY_2, 0x35),
        (TABLE_3, KEY_3, 0xBA),
    ];
    let mut data = canonical.as_bytes().to_vec();
    for (table, key, iv) in stages {
        let table = STANDARD.decode(table).expect("valid VRF table");
        let key = STANDARD.decode(key).expect("valid VRF key");
        data = encrypt_stage(&data, &table, &key, iv);
    }
    URL_SAFE_NO_PAD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::sign;

    #[test]
    fn signs_titles_query() {
        assert_eq!(
            sign("/titles?content_rating[0]=safe&content_rating[1]=suggestive&limit=30&order[chapter_updated_at]=desc&page=1"),
            "8sK3xtqdFZdOu6WNqS1bZ0shnUDqyRXMnh4NlZ7aYCPUhmAbm1C1qPzeL_OIIf0obIggCZIHJHIF_VdaYGWoz1D2WyKu2XhBqaoQcC-UzOL9vlMOE6MXU01kzYuIPwgPSvk_Z55Rw17nfA"
        );
    }
}
