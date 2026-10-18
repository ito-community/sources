# Contributing

## Reporting a broken source

Open an issue with the plugin name and version, what fails (home, search, details, chapters, or pages), and an example title or URL.

## Changing a plugin

1. Fork the repository and branch from `main`.
2. Make the change under `plugins/<name>/`.
3. Bump `version` in `manifest.json`. The index only publishes a new package when the version changes. Bump the crate version in `Cargo.toml` as well.
4. Run `ito-pkg pack` in the plugin directory and confirm it succeeds without warnings, then `ito-pkg verify <name>.ito`.
5. Test the packed `.ito` in Ito from a local repository (step 7 of [Adding a plugin](#adding-a-plugin)): home, a listing, search, details, chapters, and reading a chapter.
6. Open a pull request describing what broke, what changed, and how you tested it.

Keep each pull request to one plugin where possible.

## Adding a plugin

1. Install the tools:

   ```sh
   rustup target add wasm32-unknown-unknown
   cargo install --git https://github.com/kunihir0/ito-pkg
   ```

2. Scaffold the plugin from `plugins/`:

   ```sh
   cd plugins
   ito-pkg new <name> -t <manga|anime|novel>
   ```

   This creates `Cargo.toml` (a `cdylib` crate depending on `ito-rs`), `manifest.json`, and `src/lib.rs`.

3. Fix the generated `src/lib.rs`. The `ito-pkg` templates predate the current `ito-rs` traits and do not compile as generated. Every provider method returns `ito_rs::Result<T>`, and the search method takes `query: &str`. Copy the signatures from an existing plugin of the same type.

4. Edit `manifest.json`:
   - `id`: `com.<author>.<name>`. Do not keep `com.example`.
   - `name`, `url` (the site), `author`, `description`, `language`, `languages`.
   - `sourceUrl`: `https://github.com/ito-community/sources`.
   - `contentRating` and `nsfw` to match the site.
   - `version`: start at `1.0.0`.

5. Add `icon.png` to the plugin directory.

6. Implement the provider, then package and check it:

   ```sh
   cd plugins/<name>
   ito-pkg pack
   ito-pkg verify <name>.ito
   ```

7. Test it in Ito from a local repository:

   ```sh
   mkdir -p /tmp/ito-repo && cp plugins/<name>/<name>.ito /tmp/ito-repo/
   ito-pkg repo --input /tmp/ito-repo --output /tmp/ito-repo/public --url http://<host>:8080
   ito-pkg serve --path /tmp/ito-repo/public --port 8080
   ```

   Add `http://<host>:8080` as a repository in Ito, install the plugin, and check home, a listing, search, details, chapters, and reading. Use `localhost` for the iOS Simulator and your computer's LAN address for a device. `--url` must match the address you add in Ito.

8. Open a pull request.

## Guidelines

- Do not commit build output (`target/`, `*.ito`).
- Do not commit credentials, tokens, or cookies.
- Send only the headers a site requires, such as `Referer` for image CDNs.
