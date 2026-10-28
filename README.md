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
| Atsumaru | manga, novel | atsu.moe |
| Comix | manga | comix.to |
| HiAnime | anime | hianime.ms |
| LuaComics | manga | luacomic.org |
| MangaFire | manga | mangafire.to |
| Novel Fire | novel | novelfire.net |
| VioletScans | manga | violetmanga.com |

Comix requires Ito 1.4.0 or newer. It uses a verified version of the site's browser client;
if the site changes that client, update the source package rather than bypassing its revision
check. Site verification prompts still require user interaction.

Atsumaru requires Ito 1.4.0 or newer. Hosts with rich controls show grouped source settings,
searchable tag and genre selection, and per-search filters and sorting. Earlier hosts keep
the text-based settings and search preferences. Content, Discover and chapter preferences
are saved; per-search filters on rich hosts last only for the open source screen.
The maximum content rating starts at Safe, which also narrows tracker matching; relax it
if a title cannot be found. Ito has three content levels, so the site's Erotica and
Pornographic ratings both show as NSFW.

## Building

Requires Rust with the `wasm32-unknown-unknown` target and `ito-pkg`:

```sh
rustup target add wasm32-unknown-unknown
cargo install --force --git https://github.com/AtiinLabs/ito-pkg
cd plugins/<name>
ito-pkg pack
```

Pushes to `main` that touch `plugins/` build every plugin, regenerate `index.json`, and deploy to GitHub Pages.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
