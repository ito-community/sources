# Ito Sources

Community-maintained source plugins for [Ito](https://github.com/AtiinLabs/Ito).

## Usage

In Ito, add this repository URL:

```
https://ito-community.github.io/sources
```

## Plugins

| Plugin | Type | Site |
|---|---|---|
| Anikoto | anime | anikototv.to |
| AnimeKai | anime | animekai.to |
| Asura Scans | manga | asurascans.com |
| Atsumaru | manga | atsu.moe |
| Comix | manga | comix.to |
| LuaComics | manga | luacomic.org |
| MangaFire | manga | mangafire.to |
| Novel Fire | novel | novelfire.net |
| VioletScans | manga | violetmanga.com |

## Building

Requires Rust with the `wasm32-unknown-unknown` target and `ito-pkg`:

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/kunihir0/ito-pkg
cd plugins/<name>
ito-pkg pack
```

Pushes to `main` that touch `plugins/` build every plugin, regenerate `index.json`, and deploy to GitHub Pages.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
