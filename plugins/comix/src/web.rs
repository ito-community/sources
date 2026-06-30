use crate::BASE_URL;
use ito_rs::net::Request;
use ito_rs::webview::Webview;
use ito_rs::{Error, Result};
use regex::Regex;
use serde::Deserialize;

use std::sync::atomic::{AtomicBool, Ordering};

static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);



const INSTALLER_REQUEST_TOKEN: &str = "__AIDOKU_INSTALLER_REQUEST_TOKEN__";
const INSTALLER_RESPONSE_TOKEN: &str = "__AIDOKU_INSTALLER_RESPONSE_TOKEN__";
const DESCRAMBLER_BLOB_TOKEN: &str = "__AIDOKU_DESCRAMBLER_BLOB_TOKEN__";
const DESCRAMBLER_CANVAS_TOKEN: &str = "__AIDOKU_DESCRAMBLER_CANVAS_TOKEN__";

const DESCRAMBLER_RESPONSE_TOKEN: &str = "__AIDOKU_DESCRAMBLER_RESPONSE_TOKEN__";
const EMPTY_DESCRAMBLER_RESPONSE_OBJECT: &str =
	"{ data: null, error: null, isDone: false, isAbort: false }";
const FETCH_TIMEOUT_RESPONSE: &str =
	"Fetch timeout after 30s. If problem persist, please restart the application.";


#[derive(Deserialize)]
struct DescrambleResponseObject {
	data: Option<String>,
	error: Option<String>,
}

pub struct ComixWebView;

impl ComixWebView {
	fn sleep_ms(ms: u64) {
		let iters = ms * 500_000;
		let mut dummy: u64 = 0;
		for i in 0..iters {
			dummy = dummy.wrapping_add(i);
			std::hint::spin_loop();
		}
		let _ = std::hint::black_box(dummy);
	}

	fn load_webview() -> Result<()> {
		ito_rs::host::print("load_webview: calling Webview::load_url to set origin");
		let _ = Webview::load_url(BASE_URL);

		ito_rs::host::print("load_webview: calling find_functions");
		
		let mut timeout = 0;
		let mut found = false;
		loop {
			match Self::find_functions() {
				Ok(_) => {
					found = true;
					break;
				}
				Err(_) => {
					if timeout > 2000 {
						break;
					}
					let mut dummy: u64 = 0;
					for i in 0..20_000_000 { // ~2.8ms
						dummy = dummy.wrapping_add(i);
						std::hint::spin_loop();
					}
					let _ = std::hint::black_box(dummy);
					timeout += 1;
				}
			}
		}

		if !found {
			ito_rs::host::print("load_webview: find_functions failed after polling, trying find_secure_module_src");
			if let Err(e2) = Self::find_secure_module_src() {
				ito_rs::host::print(&format!("load_webview: find_secure_module_src failed: {:?}", e2));
				return Err(e2);
			}
			ito_rs::host::print("load_webview: find_secure_module_src succeeded, calling find_functions again");
			Self::find_functions()?;
		}

		ito_rs::host::print("load_webview: initialized successfully");
		IS_INITIALIZED.store(true, Ordering::SeqCst);
		Ok(())
	}

	fn find_secure_module_src() -> Result<()> {
		ito_rs::host::print("find_secure_module_src: fetching base html");
		let mut request = Request::get(BASE_URL);
		request.header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15");
		let response = request.send()?;
		let html = String::from_utf8_lossy(&response.body).into_owned();

		ito_rs::host::print("find_secure_module_src: matching regex");
		let main_regex = Regex::new(r#"<script[^>]*src="([^"]*main[^"]*\.js)"[^>]*>"#).map_err(|e| ito_rs::Error::Host(e.to_string()))?;
		if let Some(captures) = main_regex.captures(&html) {
			if let Some(main_module_src) = captures.get(1) {
				let src = main_module_src.as_str();
				ito_rs::host::print(&format!("find_secure_module_src: found main_module_src: {}", src));
				let full_url = if src.starts_with("http") {
					src.to_string()
				} else {
					format!("{}{}", BASE_URL.trim_end_matches('/'), src)
				};
				ito_rs::host::print(&format!("find_secure_module_src: fetching main module from: {}", full_url));
				let mut req = Request::get(&full_url);
				req.header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15");
				let res = req.send()?;
				let js = String::from_utf8_lossy(&res.body).into_owned();

				let secure_script_regex = Regex::new(r#""(secure-[^"]+\.js)""#).map_err(|e| ito_rs::Error::Host(e.to_string()))?;
				if let Some(caps) = secure_script_regex.captures(&js) {
					let secure_script_path = caps.get(1).ok_or_else(|| ito_rs::Error::Host("Regex capture missing".into()))?.as_str();
					ito_rs::host::print(&format!("find_secure_module_src: found secure_script_path: {}", secure_script_path));
					
                    let js_asset_path = if let Some(idx) = src.rfind('/') {
                        &src[0..idx + 1]
                    } else {
                        "/"
                    };
                    let js_asset_full_url = format!("{}{}", BASE_URL.trim_end_matches('/'), js_asset_path);
                    let secure_full_url = format!("{}{}", js_asset_full_url, secure_script_path);
                    ito_rs::host::print(&format!("find_secure_module_src: fetching secure script content from: {}", secure_full_url));

					let mut sreq = Request::get(&secure_full_url);
					sreq.header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15");
					let sres = sreq.send()?;
					let secure_js = String::from_utf8_lossy(&sres.body).into_owned();

					ito_rs::host::print("find_secure_module_src: evaluating secure script content via script tag");
					
					// Inject via a <script type="module"> tag to avoid SyntaxError for ES modules
                    let js_string_literal = serde_json::to_string(&secure_js).unwrap_or_else(|_| "\"\"".to_string());
					let inject_code = format!(
						"(() => {{
							try {{
								let base = document.createElement('base');
								base.href = '{}';
								document.head.appendChild(base);

								let script = document.createElement('script');
								script.type = 'module';
								script.textContent = {};
								document.head.appendChild(script);
								return 'ok';
							}} catch (e) {{
								return e.toString();
							}}
						}})()", js_asset_full_url, js_string_literal
					);

					let eval_res = Webview::execute_js(&inject_code);
					match eval_res {
						Ok(res) => ito_rs::host::print(&format!("find_secure_module_src: script injected, result: {}", res)),
						Err(_) => ito_rs::host::print("find_secure_module_src: script injection failed entirely")
					}
					
					// wait loop for vm
					ito_rs::host::print("find_secure_module_src: waiting for vm prefix");
					let mut timeout = 0;
					loop {
						match Self::find_functions() {
							Ok(_) => {
								ito_rs::host::print("find_secure_module_src: find_functions succeeded during polling");
								return Ok(());
							}
							Err(_) => {
								if timeout > 1000 { 
									ito_rs::host::print("find_secure_module_src: timed out waiting for vm prefix");
									return Err(Error::Unsupported); 
								}
								let mut dummy: u64 = 0;
								for i in 0..10_000_000 {
									dummy = dummy.wrapping_add(i);
									std::hint::spin_loop();
								}
								let _ = std::hint::black_box(dummy);
								timeout += 1;
							}
						}
					}
				} else {
					ito_rs::host::print("find_secure_module_src: could not match secure_script_regex");
				}
			}
		} else {
			ito_rs::host::print("find_secure_module_src: could not match main_regex");
		}
		Err(Error::Unsupported)
	}

    fn check_initialized() -> Result<bool> {
        Ok(IS_INITIALIZED.load(Ordering::SeqCst))
    }

	fn find_functions() -> Result<()> {
		let result = Webview::execute_js(&format!(
			"(() => {{
			try {{
                if (window._ito_inst_found && window._ito_desc_found) {{
                    return 'true';
                }}

				let inst = '', descBlob = '', descCanvas = '';
                let dbg = [];
				const isPromise = (v) => v && (typeof v === 'object' || typeof v === 'function') && typeof v.then === 'function';
				const canvas = document.createElement('canvas');
				const controller = new AbortController();
                const signal = controller.signal;
                
                const keys = Object.keys(window);
                for (let i = 0; i < keys.length; i++) {{
                    const topName = keys[i];
                    if (topName.indexOf('vm') !== 0) continue;
                    const ns = window[topName];
                    if (!ns || typeof ns !== 'object') continue;
                    const fnames = Object.keys(ns);
                    dbg.push('vm_keys=' + fnames.length);

                    for (let j = 0; j < fnames.length; j++) {{
                        let fn = ns[fnames[j]];
                        if (typeof fn !== 'function') continue;
                        let ref = 'window.' + topName + '.' + fnames[j];

                        if (!window._ito_inst_found) {{
                            try {{
                                let got = false;
                                let fakeAxios = {{
                                    interceptors: {{
                                        request:  {{ use: function() {{ got = true; }} }},
                                        response: {{ use: function() {{ got = true; }} }}
                                    }},
                                    defaults: {{ headers: {{ common: {{}} }}, transformRequest: [], transformResponse: [] }}
                                }};

                                fn(fakeAxios);
                                if (got) {{
                                    inst = ref;
                                    fn({{
                                        interceptors: {{
                                            request: {{
                                                use: function (handler) {{ window['{INSTALLER_REQUEST_TOKEN}'] = handler; }},
                                            }},
                                            response: {{
                                                use: function (handler) {{ window['{INSTALLER_RESPONSE_TOKEN}'] = handler; }},
                                            }},
                                        }},
                                        defaults: {{ headers: {{ common: {{}} }}, transformRequest: [], transformResponse: [] }}
                                    }});
                                    window._ito_inst_found = true;
                                }}
                            }} catch (e) {{}}
                        }}
                        if (!window._ito_desc_found) {{
                            try {{
                                if (fn.length == 3) {{
                                    let p = fn('https://comix.to/test.jpg', canvas, signal);
                                    if (isPromise(p)) {{
                                        descCanvas = ref;
                                        window['{DESCRAMBLER_CANVAS_TOKEN}'] = fn;
                                        window._ito_desc_found = true;
                                    }}
                                }}
                            }} catch (e) {{}}
                        }}
                        if (!window._ito_desc_found) {{
                            try {{
                                if (fn.length == 2) {{
                                    let p = fn('https://comix.to/test.jpg', signal);
                                    if (isPromise(p)) {{
                                        descBlob = ref;
                                        window['{DESCRAMBLER_BLOB_TOKEN}'] = fn;
                                        window._ito_desc_found = true;
                                    }}
                                }}
                            }} catch (e) {{}}
                        }}
                    }}
                }}
                
                if (window._ito_inst_found && window._ito_desc_found) {{
                    return 'true';
                }}
                let debugInfo = 'missing:' + [
                    window._ito_inst_found ? '' : 'inst',
                    window._ito_desc_found ? '' : 'desc'
                ].filter(Boolean).join(',');
                return debugInfo;
			}} catch (e) {{
				return 'error: ' + e;
			}}
		}})()",
		))?;
		
		ito_rs::host::print(&format!("find_functions check: {}", result));
		
		if result == "true" || result == "\"true\"" {
            Ok(())
        } else {
            Err(Error::Unsupported)
        }
	}

	pub fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
		ito_rs::host::print(&format!("fetch_json called for URL: {}", url));
		if !Self::check_initialized().unwrap_or(false) {
			ito_rs::host::print("fetch_json: webview not initialized, loading...");
			if let Err(e) = Self::load_webview() {
				ito_rs::host::print(&format!("fetch_json: failed to load webview: {:?}", e));
				return Err(e);
			}
		}

		ito_rs::host::print(&format!("fetch_json: building request for {}", url));

		let result = match Webview::execute_js(&format!(
			"(() => {{
			try {{
				const url = new URL('{}');
				const result = {{}};

				for (const [key, rawValue] of url.searchParams) {{
					const value = /^\\d+$/.test(rawValue) ? Number(rawValue) : rawValue;
					const parts = key.replace(/\\]/g, '').split('[');
					let current = result;
					for (let i = 0; i < parts.length; i++) {{
						const part = parts[i];
						const last = i === parts.length - 1;
						if (last) {{
							if (part === '') {{
								current.push(value);
							}} else if (current[part] === undefined) {{
								current[part] = value;
							}} else if (Array.isArray(current[part])) {{
								current[part].push(value);
							}} else {{
								current[part] = [current[part], value];
							}}
						}} else {{
							const nextPart = parts[i + 1];
							current[part] ??= nextPart === '' ? [] : {{}};
							current = current[part];
						}}
					}}
				}}

				const request = window['{}']({{
					url: `${{url.origin}}${{url.pathname}}`,
					method: 'GET',
					params: result,
				}});

				const finalUrl = new URL(request.url);
				function appendParams(params, prefix) {{
					for (const key in params) {{
						const value = params[key];
						if (value === null || value === undefined) continue;
						const newKey = prefix ? `${{prefix}}[${{key}}]` : key;
						if (Array.isArray(value)) {{
							value.forEach(v => finalUrl.searchParams.append(`${{newKey}}[]`, v));
						}} else if (typeof value === 'object') {{
							appendParams(value, newKey);
						}} else {{
							finalUrl.searchParams.append(newKey, value);
						}}
					}}
				}}
				if (request.params) {{
					appendParams(request.params, '');
				}}
				return finalUrl.toString();
			}} catch (e) {{
				return 'error: ' + e;
			}}
		}})()", url, INSTALLER_REQUEST_TOKEN
		)) {
			Ok(r) => r,
			Err(e) => {
				ito_rs::host::print(&format!("fetch_json: execute_js for signer failed: {:?}", e));
				return Err(e);
			}
		};

		if result.starts_with("error:") || result.is_empty() {
			ito_rs::host::print(&format!("fetch_json: failed to generate signature: {}", result));
			return Err(Error::Unsupported);
		}

		let signed_url = result.trim_matches('"').to_string();
		ito_rs::host::print(&format!("fetch_json: executing signed request: {}", signed_url));
		let mut req = Request::get(&signed_url);
        req.header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15");

		let response = match req.send() {
			Ok(r) => r,
			Err(e) => {
				ito_rs::host::print(&format!("fetch_json: network request failed: {:?}", e));
				return Err(e);
			}
		};

		if let Some(x_enc) = response.headers.get("x-enc") {
			ito_rs::host::print(&format!("fetch_json: x-enc header found: {}", x_enc));
			if x_enc == "1" {
				let encoded_response = String::from_utf8_lossy(&response.body)
					.replace("\\", "\\\\")
					.replace("'", "\\'");

				let result = match Webview::execute_js(&format!(
					"(() => {{
						try {{
							let decoded = window['{INSTALLER_RESPONSE_TOKEN}']({{
								data: JSON.parse('{encoded_response}'),
								status: 200,
								headers: {{
									'x-enc': '1',
								}},
							}});
							return JSON.stringify({{ result: decoded && decoded.data }});
						}} catch(e) {{
							return 'error: ' + e;
						}}
					}})()",
				)) {
					Ok(r) => r,
					Err(e) => {
						ito_rs::host::print(&format!("fetch_json: execute_js for decode failed: {:?}", e));
						return Err(e);
					}
				};

				ito_rs::host::print(&format!("fetch_json: decode result: {}", result));

				if result.starts_with("error:") || result.is_empty() {
					ito_rs::host::print(&format!("fetch_json: decode result had error or was empty: {}", result));
					return Err(Error::Unsupported);
				}
				
                let unquoted = if result.starts_with('"') && result.ends_with('"') {
                    result[1..result.len()-1].replace("\\\"", "\"").replace("\\\\", "\\")
                } else {
                    result.clone()
                };

                let wrapper: serde_json::Value = match serde_json::from_str(&unquoted) {
					Ok(r) => r,
					Err(e) => {
						ito_rs::host::print(&format!("fetch_json: failed to parse decoded wrapper: {:?}", e));
						return Err(Error::Unsupported);
					}
				};

                // Try parsing the wrapper directly first, as it has the shape {"result": decoded.data}.
                // If decoded.data is already just {"items": ...}, then wrapper perfectly matches ChapterDetailsResponse!
                if let Ok(r) = serde_json::from_value(wrapper.clone()) {
                    ito_rs::host::print("fetch_json: successfully parsed target json from wrapper");
                    return Ok(r);
                }

                if let Some(res_data) = wrapper.get("result") {
                    return match serde_json::from_value(res_data.clone()) {
						Ok(r) => {
							ito_rs::host::print("fetch_json: successfully parsed target json from wrapper.result");
							Ok(r)
						},
						Err(e) => {
							ito_rs::host::print(&format!("fetch_json: failed to deserialize target type: {:?}", e));
							Err(Error::Unsupported)
						}
					};
                }
				ito_rs::host::print("fetch_json: missing result key in wrapper");
                return Err(Error::Unsupported);
			}
		}

		ito_rs::host::print("fetch_json: no x-enc or not 1, trying direct parse");
		let body_str = String::from_utf8_lossy(&response.body);
		ito_rs::host::print(&format!("fetch_json: raw response body: {}", &body_str[..body_str.len().min(500)]));
		match serde_json::from_slice(&response.body) {
			Ok(r) => Ok(r),
			Err(e) => {
				ito_rs::host::print(&format!("fetch_json: direct parse failed: {:?}", e));
				Err(Error::Unsupported)
			}
		}
	}

	pub fn descramble_image(width: f32, height: f32, url: &str) -> Result<String> {
		if !Self::check_initialized().unwrap_or(false) {
			Self::load_webview()?;
		}

		ito_rs::host::print(&format!("descramble_image: natively fetching {}", url));
		let mut req = Request::get(url);
		req.header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15");
		req.header("Referer", BASE_URL);
		let res = req.send()?;
		
		let headers_json = serde_json::to_string(&res.headers).unwrap_or_else(|_| "{}".to_string());
		let image_bytes = res.body;
		
		use base64::{Engine as _, engine::general_purpose};
		let b64 = general_purpose::STANDARD.encode(&image_bytes);
		ito_rs::host::print(&format!("descramble_image: natively fetched {} bytes -> {} b64", image_bytes.len(), b64.len()));

		// First, check which descramblers are available
		let check = Webview::execute_js(&format!(
			"(() => {{
				const hasBlob = window['{DESCRAMBLER_BLOB_TOKEN}'] != null;
				const hasCanvas = window['{DESCRAMBLER_CANVAS_TOKEN}'] != null;
				return JSON.stringify({{ hasBlob, hasCanvas }});
			}})()"
		));
		match &check {
			Ok(r) => ito_rs::host::print(&format!("descramble_image: descramblers available: {}", r)),
			Err(e) => ito_rs::host::print(&format!("descramble_image: check failed: {:?}", e)),
		}

		ito_rs::host::print(&format!("descramble_image: url={}, w={}, h={}", url, width, height));

		let mut lock_timeout = 0;
		loop {
			let res = Webview::execute_js(&format!(
				"(() => {{
					if (window._ito_is_descrambling) {{
						return 'locked';
					}}
					window._ito_is_descrambling = true;
					window._ito_debug = [];
					window['{DESCRAMBLER_RESPONSE_TOKEN}'] = {EMPTY_DESCRAMBLER_RESPONSE_OBJECT};

					// Catch any unhandled promise rejections
					window.addEventListener('unhandledrejection', (event) => {{
						window._ito_debug.push('UNHANDLED_REJECTION: ' + (event.reason ? event.reason.message || String(event.reason) : 'unknown'));
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = 'Unhandled rejection: ' + (event.reason ? event.reason.message || String(event.reason) : 'unknown');
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
						window._ito_is_descrambling = false;
					}}, {{ once: true }});

					const canvas = document.createElement('canvas');
					canvas.width = {width};
					canvas.height = {height};

					let cleanToDataURL = HTMLCanvasElement.prototype.toDataURL;
					try {{
						const iframe = document.createElement('iframe');
						iframe.style.display = 'none';
						document.documentElement.appendChild(iframe);
						if (iframe.contentWindow && iframe.contentWindow.HTMLCanvasElement) {{
							cleanToDataURL = iframe.contentWindow.HTMLCanvasElement.prototype.toDataURL;
						}}
						document.documentElement.removeChild(iframe);
					}} catch (e) {{
						window._ito_debug.push('IFRAME_ERR: ' + e);
					}}

					const controller = new AbortController();
					const signal = controller.signal;

					const timeout = setTimeout(() => {{
						window._ito_debug.push('TIMEOUT_FIRED');
						controller.abort();
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].isAbort = true;
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = '{FETCH_TIMEOUT_RESPONSE}';
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
						window._ito_is_descrambling = false;
					}}, 30000);

					// Natively fetched bytes injected as base64
					let mockBlob = null;
					try {{
						const b64 = '{}';
						const binaryStr = atob(b64);
						const len = binaryStr.length;
						const bytes = new Uint8Array(len);
						for (let i = 0; i < len; i++) {{
							bytes[i] = binaryStr.charCodeAt(i);
						}}
						mockBlob = new Blob([bytes]);
						window._ito_debug.push('CREATED_MOCK_BLOB');
					}} catch(e) {{
						window._ito_debug.push('BLOB_CREATE_ERR: ' + e);
					}}

					const originalFetch = window.fetch;
					window.fetch = function(fetchUrl, options) {{
						const urlStr = typeof fetchUrl === 'string' ? fetchUrl : (fetchUrl instanceof URL ? fetchUrl.toString() : (fetchUrl.url || ''));
						if (urlStr === '{url}' || urlStr.includes('{url}')) {{
							window._ito_debug.push('INTERCEPTED_FETCH: ' + urlStr);
							const mockHeaders = {headers_json};
							const resp = new Response(mockBlob, {{ status: 200, statusText: 'OK', headers: mockHeaders }});
							const proxyResp = new Proxy(resp, {{
								get(target, prop) {{
									if (prop === 'url') return urlStr;
									if (prop === 'clone') return () => new Proxy(target.clone(), this);
									const val = target[prop];
									return typeof val === 'function' ? val.bind(target) : val;
								}}
							}});
							return Promise.resolve(proxyResp);
						}}
						return originalFetch.apply(this, arguments);
					}};

					// TEST FETCH
					fetch('{url}', {{ signal }}).then(r => {{
						window._ito_debug.push('TEST_FETCH_STATUS=' + r.status);
					}}).catch(e => {{
						window._ito_debug.push('TEST_FETCH_ERROR=' + e.message);
					}});

					if (window['{DESCRAMBLER_BLOB_TOKEN}'] != null) {{
						window._ito_debug.push('BLOB_PATH_START');
						try {{
							window['{DESCRAMBLER_BLOB_TOKEN}']('{url}', signal)
								.then((data) => {{
									window._ito_debug.push('BLOB_THEN: type=' + typeof data + ' mode=' + (data && data.mode));
									if (typeof data === 'object' && data.mode) {{
										if (data.mode === 'blob') {{
										window._ito_debug.push('BLOB_MODE_BLOB');
										return new Promise((resolve, reject) => {{
											const url = URL.createObjectURL(data.blob);
											const image = new Image();
											image.src = url;
											image.onload = () => resolve(image);
											image.onerror = reject;
										}})
									}} else if (data.mode === 'canvas') {{
										window._ito_debug.push('BLOB_MODE_CANVAS');
										const applyRes = data.apply(canvas);
										if (applyRes && typeof applyRes.then === 'function') {{
											window._ito_debug.push('BLOB_MODE_CANVAS_PROMISE');
											return applyRes.then(() => {{
												const output = cleanToDataURL.call(canvas);
												window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = output;
												window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
												window._ito_is_descrambling = false;
												clearTimeout(timeout);
												return null;
											}});
										}} else {{
											const output = cleanToDataURL.call(canvas);
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = output;
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
											window._ito_is_descrambling = false;
											clearTimeout(timeout);
											return null;
										}}
									}} else {{
										throw new Error('Unknown data mode');
									}}
								}} else if (typeof data === 'object' && data.apply && typeof data.apply === 'function') {{
									window._ito_debug.push('BLOB_APPLY');
									const applyRes = data.apply(canvas);
									if (applyRes && typeof applyRes.then === 'function') {{
										window._ito_debug.push('BLOB_APPLY_PROMISE');
										return applyRes.then(() => {{
											const output = cleanToDataURL.call(canvas);
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = output;
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
											window._ito_is_descrambling = false;
											clearTimeout(timeout);
											return null;
										}});
									}} else {{
										const output = cleanToDataURL.call(canvas);
										window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = output;
										window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
										window._ito_is_descrambling = false;
										clearTimeout(timeout);
										return null;
									}}
								}} else if (typeof data === 'object' && data.blob) {{
									window._ito_debug.push('BLOB_DATA_BLOB');
									return new Promise((resolve, reject) => {{
										const url = URL.createObjectURL(data.blob);
										const image = new Image();
										image.src = url;
										image.onload = () => resolve(image);
										image.onerror = reject;
									}})
								}} else {{
									window._ito_debug.push('BLOB_DATA_OTHER');
									return new Promise((resolve, reject) => {{
										const url = URL.createObjectURL(data);
										const image = new Image();
										image.src = url;
										image.onload = () => resolve(image);
										image.onerror = reject;
									}})
								}}
							}})
							.then((obj) => {{
								window._ito_debug.push('SECOND_THEN: obj=' + (obj == null ? 'null' : typeof obj));
								if (typeof obj === 'object' && obj) {{
									URL.revokeObjectURL(obj.src);
									const ctx = canvas.getContext('2d');
									canvas.width = obj.naturalWidth || obj.width || {width};
									canvas.height = obj.naturalHeight || obj.height || {height};
									ctx.drawImage(obj, 0, 0);
									const data = cleanToDataURL.call(canvas);
									window._ito_debug.push('DRAWN_TODATAURL_LEN=' + data.length);
									window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = data;
									window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
									window._ito_is_descrambling = false;
									clearTimeout(timeout);
								}}
							}})
							.catch((error) => {{
								window._ito_debug.push('BLOB_CATCH: ' + (error && error.message || error));
								if (window['{DESCRAMBLER_CANVAS_TOKEN}'] != null) {{
									window._ito_debug.push('FALLBACK_TO_CANVAS');
									window['{DESCRAMBLER_CANVAS_TOKEN}']('{url}', canvas, signal)
										.then(() => {{
											window._ito_debug.push('CANVAS_FALLBACK_THEN');
											const data = cleanToDataURL.call(canvas);
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = data;
										}})
										.catch((error) => {{
											window._ito_debug.push('CANVAS_FALLBACK_CATCH: ' + (error && error.message || error));
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = error.message || String(error);
										}})
										.finally(() => {{
											window._ito_debug.push('CANVAS_FALLBACK_FINALLY');
											window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
											window._ito_is_descrambling = false;
											clearTimeout(timeout);
										}});
								}} else {{
									window._ito_debug.push('NO_CANVAS_FALLBACK');
									window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = error.message || String(error);
									window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
									window._ito_is_descrambling = false;
									clearTimeout(timeout);
								}}
							}});
					}} catch (syncErr) {{
						window._ito_debug.push('SYNC_ERROR: ' + syncErr.message);
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = syncErr.message;
						window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
						window._ito_is_descrambling = false;
						clearTimeout(timeout);
					}}
				}} else if (window['{DESCRAMBLER_CANVAS_TOKEN}'] != null) {{
					window._ito_debug.push('CANVAS_PATH_START');
					window['{DESCRAMBLER_CANVAS_TOKEN}']('{url}', canvas, signal)
						.then(() => {{
							window._ito_debug.push('CANVAS_THEN');
							const data = cleanToDataURL.call(canvas);
							window['{DESCRAMBLER_RESPONSE_TOKEN}'].data = data;
						}})
						.catch((error) => {{
							window._ito_debug.push('CANVAS_CATCH: ' + (error && error.message || error));
							window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = error.message || String(error);
						}})
						.finally(() => {{
							window._ito_debug.push('CANVAS_FINALLY');
							window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
							window._ito_is_descrambling = false;
							clearTimeout(timeout);
						}});
				}} else {{
					window._ito_debug.push('NO_DESCRAMBLER');
					window['{DESCRAMBLER_RESPONSE_TOKEN}'].error = 'No suitable descrambler found.';
					window['{DESCRAMBLER_RESPONSE_TOKEN}'].isDone = true;
					window._ito_is_descrambling = false;
					clearTimeout(timeout);
				}}
					return 'ok';
				}})()",
				b64
			))?;

			if res == "locked" {
				if lock_timeout > 600 { return Err(Error::Unsupported); }
				Self::sleep_ms(100);
				lock_timeout += 1;
				continue;
			}
			break;
		}

		let mut timeout = 0;
		loop {
			// Single combined JS call to check status + read debug log
			let status = Webview::execute_js(&format!(
				"(() => {{
					const r = window['{DESCRAMBLER_RESPONSE_TOKEN}'];
					const d = window._ito_debug || [];
					return JSON.stringify({{
						done: !!r.isDone,
						abort: !!r.isAbort,
						debug: d
					}});
				}})()"
			))?;

			// Parse the combined status
			let unquoted = if status.starts_with('"') && status.ends_with('"') {
				status[1..status.len()-1].replace("\\\"", "\"").replace("\\\\", "\\")
			} else {
				status.clone()
			};
			
			#[derive(serde::Deserialize)]
			struct PollStatus {
				done: bool,
				abort: bool,
				debug: Vec<String>,
			}
			
			if let Ok(ps) = serde_json::from_str::<PollStatus>(&unquoted) {
				if timeout % 6 == 0 && !ps.debug.is_empty() {
					ito_rs::host::print(&format!("descramble_image: poll {} debug={:?}", timeout, ps.debug));
				}
				
				if ps.done {
					ito_rs::host::print(&format!("descramble_image: DONE at poll {} debug={:?}", timeout, ps.debug));
					break;
				}
				
				if ps.abort {
					ito_rs::host::print(&format!("descramble_image: ABORT debug={:?}", ps.debug));
					let _ = Self::load_webview();
					return Err(Error::Unsupported);
				}
			}
			
			if timeout > 120 { // 120 * 500ms = 60s
				ito_rs::host::print("descramble_image: TIMEOUT after 60s");
				let _ = Webview::execute_js("(() => { window._ito_is_descrambling = false; return ''; })()");
				return Err(Error::Unsupported);
			}
			
			// 500ms between polls to give the event loop breathing room for fetch()
			Self::sleep_ms(500);
			timeout += 1;
		}


		let result = Webview::execute_js(&format!(
			"(() => {{ return JSON.stringify(window['{DESCRAMBLER_RESPONSE_TOKEN}']); }})()"
		))?;
        
        let unquoted = if result.starts_with('"') && result.ends_with('"') {
            result[1..result.len()-1].replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            result.clone()
        };

		let json = serde_json::from_str::<DescrambleResponseObject>(&unquoted)
			.unwrap_or(DescrambleResponseObject { data: None, error: None });

		if let Some(error) = json.error {
			ito_rs::host::print(&format!("JS descrambler error: {}", error));
			return Err(Error::Unsupported);
		}

		if let Some(data) = json.data {
			return Ok(data);
		}

		Err(Error::Unsupported)
	}
}