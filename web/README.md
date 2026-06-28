# web/ — kobeMC download page

Static download page (`index.html`) for the kobeMC launcher. Pure HTML/CSS/JS,
no build step: it fetches `releases/latest` from the GitHub API at runtime, so it
always points at the newest release without redeploys.

## Deploy

Vercel project with **Root Directory = `web`** (the rest of the repo is the Rust
app + backend and is not part of this site). Auto-deploys on push to `main`;
`vercel.json`'s `ignoreCommand` skips the build when nothing under `web/` changed,
so Rust-only commits don't redeploy the page.

Live: https://mc.sma1lboy.me  ·  Local preview: `open web/index.html`
