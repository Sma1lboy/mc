#!/usr/bin/env python3
"""Generate the kobeMC README banners (dark/workspace + light/classic).

The right side shows the real brand logo (docs/assets/brand/iso-cube@1024.png),
base64-embedded so headless Chrome renders it without file-access flags.

Writes banner-<theme>.html next to this file; render to PNG with headless Chrome:

  CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
  for t in dark light; do
    "$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=2 \
      --window-size=1280,360 --screenshot=.github/assets/banner-$t.png \
      "file://$PWD/.github/assets/banner-$t.html"
  done
"""
import base64
import os

THEMES = {
    "dark": dict(
        bg="radial-gradient(120% 140% at 12% -10%, #16203a 0%, #0d1117 46%, #090c12 100%)",
        glow="radial-gradient(40% 80% at 82% 30%, rgba(53,192,125,.18), transparent 70%)",
        card="rgba(255,255,255,.04)", cardb="rgba(255,255,255,.09)",
        text="#e9eef5", dim="#8a97a8",
        accent="#35c07d", accent2="#62da9d",
        halo="rgba(53,192,125,.30)",
        chip="rgba(255,255,255,.05)", chipb="rgba(255,255,255,.10)", chipt="#c4cdda",
    ),
    "light": dict(
        bg="radial-gradient(120% 140% at 12% -10%, #eaf1fd 0%, #f4f8fc 50%, #ffffff 100%)",
        glow="radial-gradient(40% 80% at 82% 30%, rgba(19,112,243,.14), transparent 70%)",
        card="rgba(19,112,243,.04)", cardb="rgba(19,112,243,.14)",
        text="#16202e", dim="#5d6b7e",
        accent="#1370f3", accent2="#4890f5",
        halo="rgba(19,112,243,.22)",
        chip="rgba(19,112,243,.07)", chipb="rgba(19,112,243,.18)", chipt="#3a567c",
    ),
}

TEMPLATE = """<!doctype html><html><head><meta charset="utf-8"><style>
  * {{ margin: 0; box-sizing: border-box; }}
  html, body {{ width: 1280px; height: 360px; }}
  body {{
    font-family: -apple-system, "PingFang SC", "Segoe UI", system-ui, sans-serif;
    background: {bg};
    color: {text};
    -webkit-font-smoothing: antialiased;
    position: relative; overflow: hidden;
  }}
  .glow {{ position: absolute; inset: 0; background: {glow}; }}
  /* faint inner frosted border for the glass feel */
  .frame {{ position: absolute; inset: 14px; border: 1px solid {cardb};
            border-radius: 22px; background: {card}; backdrop-filter: blur(2px); }}
  .wrap {{ position: absolute; inset: 14px; display: flex; align-items: center;
           padding: 0 60px; gap: 40px; }}
  .left {{ flex: 1; min-width: 0; }}
  .eyebrow {{ font-size: 14px; letter-spacing: 4px; font-weight: 700;
              color: {dim}; text-transform: uppercase; }}
  .word {{ font-size: 94px; font-weight: 800; letter-spacing: -2px; line-height: 1.02;
           margin-top: 10px; }}
  .word .mc {{ color: {accent}; }}
  .tag-cn {{ font-size: 23px; font-weight: 600; margin-top: 14px; }}
  .tag-en {{ font-size: 15px; color: {dim}; margin-top: 5px; letter-spacing: .2px; }}
  .chips {{ display: flex; gap: 10px; margin-top: 22px; }}
  .chip {{ font-size: 13px; font-weight: 600; color: {chipt};
           background: {chip}; border: 1px solid {chipb};
           padding: 7px 14px; border-radius: 999px; }}
  .logo {{ position: relative; width: 248px; height: 248px; flex: 0 0 248px;
           display: flex; align-items: center; justify-content: center; }}
  .logo::before {{ content: ""; position: absolute; width: 300px; height: 300px;
                   border-radius: 50%; background: {halo}; filter: blur(46px); }}
  .logo img {{ position: relative; width: 230px; height: 230px;
               filter: drop-shadow(0 18px 34px rgba(0,0,0,.34)); }}
</style></head><body>
  <div class="glow"></div>
  <div class="frame"></div>
  <div class="wrap">
    <div class="left">
      <div class="eyebrow">Minecraft Launcher</div>
      <div class="word">kobe<span class="mc">MC</span></div>
      <div class="tag-cn">从零打造的跨平台 Minecraft 启动器</div>
      <div class="tag-en">A from-scratch, cross-platform Minecraft launcher</div>
      <div class="chips">
        <span class="chip">Rust core</span>
        <span class="chip">Tauri v2</span>
        <span class="chip">SolidJS</span>
      </div>
    </div>
    <div class="logo"><img src="data:image/png;base64,{logo}" alt="kobeMC" /></div>
  </div>
</body></html>"""


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    repo = os.path.dirname(os.path.dirname(here))
    logo_path = os.path.join(repo, "docs", "assets", "brand", "iso-cube@1024.png")
    with open(logo_path, "rb") as f:
        logo = base64.b64encode(f.read()).decode("ascii")
    for name, t in THEMES.items():
        html = TEMPLATE.format(logo=logo, **t)
        out = os.path.join(here, f"banner-{name}.html")
        with open(out, "w", encoding="utf-8") as f:
            f.write(html)
        print("wrote", out)


if __name__ == "__main__":
    main()
