# Contributing

## Reporting a broken source

Open an issue with the plugin name and version, what fails (home, search, details, chapters, or pages), and an example title or URL.

## Changing a plugin

1. Fork the repository and branch from `main`.
2. Make the change under `plugins/<name>/`.
3. Bump `version` in `manifest.json`. The index only publishes a new package when the version changes. Bump the crate version in `Cargo.toml` as well.
4. Run `ito-pkg pack` in the plugin directory and confirm it succeeds without warnings.
5. Test the packed `.ito` in Ito: home, a listing, search, details, chapters, and reading a chapter.
6. Open a pull request describing what broke, what changed, and how you tested it.

Keep each pull request to one plugin where possible.

## Adding a plugin

Create `plugins/<name>/` with:

- `manifest.json`: `id` (`com.<author>.<name>`), `name`, `version`, `type` (`manga`, `anime`, or `novel`), `language`, `url`, and `author`.
- `Cargo.toml`: a `cdylib` crate depending on `ito-rs`.
- `src/lib.rs`: the provider implementation.
- `icon.png`: the source icon.

Use an existing plugin of the same type as a template.

## Guidelines

- Do not commit build output (`target/`, `*.ito`).
- Do not commit credentials, tokens, or cookies.
- Send only the headers a site requires, such as `Referer` for image CDNs.
