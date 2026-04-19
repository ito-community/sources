use base64::{Engine, engine::general_purpose};

pub struct VrfGenerator;

impl VrfGenerator {
	fn atob(data: &str) -> Vec<u8> {
		general_purpose::STANDARD.decode(data).unwrap()
	}

	fn btoa(data: &[u8]) -> String {
		general_purpose::STANDARD.encode(data)
	}

	fn rc4(key: &[u8], input: &[u8]) -> Vec<u8> {
		let mut s: Vec<u8> = (0..=255).collect();
		let mut j = 0usize;

		// KSA
		for i in 0..256 {
			j = (j + s[i] as usize + key[i % key.len()] as usize) & 0xFF;
			s.swap(i, j);
		}

		// PRGA
		let mut output = vec![0u8; input.len()];
		let mut i = 0usize;
		j = 0usize;
		for (y, &inp) in input.iter().enumerate() {
			i = (i + 1) & 0xFF;
			j = (j + s[i] as usize) & 0xFF;
			s.swap(i, j);
			let k = s[(s[i] as usize + s[j] as usize) & 0xFF];
			output[y] = inp ^ k;
		}
		output
	}

	fn transform(
		input: &[u8],
		init_seed_bytes: &[u8],
		prefix_key_bytes: &[u8],
		prefix_len: usize,
		schedule: &[fn(u8) -> u8],
	) -> Vec<u8> {
		let mut out = Vec::new();
		for i in 0..input.len() {
			if i < prefix_len {
				out.push(prefix_key_bytes[i]);
			}
			let transformed = schedule[i % 10](input[i] ^ init_seed_bytes[i % 32]);
			out.push(transformed);
		}
		out
	}

	pub fn generate(input: &str) -> String {
		let rc4_keys: [&str; 5] = [
			"FgxyJUQDPUGSzwbAq/ToWn4/e8jYzvabE+dLMb1XU1o=",
			"CQx3CLwswJAnM1VxOqX+y+f3eUns03ulxv8Z+0gUyik=",
			"fAS+otFLkKsKAJzu3yU+rGOlbbFVq+u+LaS6+s1eCJs=",
			"Oy45fQVK9kq9019+VysXVlz1F9S1YwYKgXyzGlZrijo=",
			"aoDIdXezm2l3HrcnQdkPJTDT8+W6mcl2/02ewBHfPzg=",
		];

		let seeds32: [&str; 5] = [
			"yH6MXnMEcDVWO/9a6P9W92BAh1eRLVFxFlWTHUqQ474=",
			"RK7y4dZ0azs9Uqz+bbFB46Bx2K9EHg74ndxknY9uknA=",
			"rqr9HeTQOg8TlFiIGZpJaxcvAaKHwMwrkqojJCpcvoc=",
			"/4GPpmZXYpn5RpkP7FC/dt8SXz7W30nUZTe8wb+3xmU=",
			"wsSGSBXKWA9q1oDJpjtJddVxH+evCfL5SO9HZnUDFU8=",
		];

		let prefix_keys: [&str; 5] = [
			"l9PavRg=",
			"Ml2v7ag1Jg==",
			"i/Va0UxrbMo=",
			"WFjKAHGEkQM=",
			"5Rr27rWd",
		];

		let schedule_0: [fn(u8) -> u8; 10] = [
			|c| c.wrapping_sub(223),
			|c| c.rotate_right(4),
			|c| c.rotate_right(4),
			|c| c.wrapping_add(234),
			|c| c.rotate_right(7),
			|c| c.rotate_right(2),
			|c| c.rotate_right(7),
			|c| c.wrapping_sub(223),
			|c| c.rotate_right(7),
			|c| c.rotate_right(6),
		];
		let schedule_1: [fn(u8) -> u8; 10] = [
			|c| c.wrapping_add(19),
			|c| c.rotate_right(7),
			|c| c.wrapping_add(19),
			|c| c.rotate_right(6),
			|c| c.wrapping_add(19),
			|c| c.rotate_right(1),
			|c| c.wrapping_add(19),
			|c| c.rotate_right(6),
			|c| c.rotate_right(7),
			|c| c.rotate_right(4),
		];
		let schedule_2: [fn(u8) -> u8; 10] = [
			|c| c.wrapping_sub(223),
			|c| c.rotate_right(1),
			|c| c.wrapping_add(19),
			|c| c.wrapping_sub(223),
			|c| c.rotate_left(2),
			|c| c.wrapping_sub(223),
			|c| c.wrapping_add(19),
			|c| c.rotate_left(1),
			|c| c.rotate_left(2),
			|c| c.rotate_left(1),
		];
		let schedule_3: [fn(u8) -> u8; 10] = [
			|c| c.wrapping_add(19),
			|c| c.rotate_left(1),
			|c| c.rotate_left(1),
			|c| c.rotate_right(1),
			|c| c.wrapping_add(234),
			|c| c.rotate_left(1),
			|c| c.wrapping_sub(223),
			|c| c.rotate_left(6),
			|c| c.rotate_left(4),
			|c| c.rotate_left(1),
		];
		let schedule_4: [fn(u8) -> u8; 10] = [
			|c| c.rotate_right(1),
			|c| c.rotate_left(1),
			|c| c.rotate_left(6),
			|c| c.rotate_right(1),
			|c| c.rotate_left(2),
			|c| c.rotate_right(4),
			|c| c.rotate_left(1),
			|c| c.rotate_left(1),
			|c| c.wrapping_sub(223),
			|c| c.rotate_left(2),
		];

        // Simplified urlencode as aidoku's encode_uri_component might be complex
		let input_encoded = url_encode(input);
		let mut bytes = input_encoded.as_bytes().to_vec();

		bytes = Self::rc4(&Self::atob(rc4_keys[0]), &bytes);
		bytes = Self::transform(
			&bytes,
			&Self::atob(seeds32[0]),
			&Self::atob(prefix_keys[0]),
			Self::atob(prefix_keys[0]).len(),
			&schedule_0,
		);
		bytes = Self::rc4(&Self::atob(rc4_keys[1]), &bytes);
		bytes = Self::transform(
			&bytes,
			&Self::atob(seeds32[1]),
			&Self::atob(prefix_keys[1]),
			Self::atob(prefix_keys[1]).len(),
			&schedule_1,
		);
		bytes = Self::rc4(&Self::atob(rc4_keys[2]), &bytes);
		bytes = Self::transform(
			&bytes,
			&Self::atob(seeds32[2]),
			&Self::atob(prefix_keys[2]),
			Self::atob(prefix_keys[2]).len(),
			&schedule_2,
		);
		bytes = Self::rc4(&Self::atob(rc4_keys[3]), &bytes);
		bytes = Self::transform(
			&bytes,
			&Self::atob(seeds32[3]),
			&Self::atob(prefix_keys[3]),
			Self::atob(prefix_keys[3]).len(),
			&schedule_3,
		);
		bytes = Self::rc4(&Self::atob(rc4_keys[4]), &bytes);
		bytes = Self::transform(
			&bytes,
			&Self::atob(seeds32[4]),
			&Self::atob(prefix_keys[4]),
			Self::atob(prefix_keys[4]).len(),
			&schedule_4,
		);

		let mut encoded = Self::btoa(&bytes);
		encoded = encoded.replace("+", "-").replace("/", "_").replace("=", "");
		encoded
	}
}

fn url_encode(input: &str) -> String {
    let mut output = String::new();
    for b in input.as_bytes() {
        match *b as char {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => output.push(*b as char),
            ' ' => output.push('+'),
            _ => output.push_str(&format!("%{:02X}", b)),
        }
    }
    output
}
