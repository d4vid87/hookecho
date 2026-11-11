"""Run after npm run build: verify the shared design and local navigation."""
from html.parser import HTMLParser
import json
from pathlib import Path
import re
from urllib.parse import unquote, urljoin, urlsplit


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids, self.links, self.classes, self.frames, self.media = set(), [], [], [], []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "html":
            self.classes = attrs.get("class", "").split()
            self.palette = attrs.get("data-palette")
            self.design = attrs.get("data-design")
        if tag == "meta" and attrs.get("name") == "robots":
            self.robots = attrs.get("content", "")
        if tag == "a" and attrs.get("href"):
            self.links.append(attrs["href"])
        if tag == "iframe":
            self.frames.append(attrs.get("src", ""))
        if tag in {"img", "video", "source"}:
            self.media.extend(attrs[key] for key in ("src", "data-src", "poster") if attrs.get(key))
            self.media.extend(item.strip().split()[0] for item in attrs.get("srcset", "").split(",") if item.strip())
        if tag == "meta" and (attrs.get("property") == "og:image" or attrs.get("name") == "twitter:image"):
            self.media.append(attrs.get("content", ""))
        if "data-features" in attrs:
            for feature in json.loads(attrs["data-features"]):
                self.media.extend(feature[key] for key in ("src", "poster"))


dist = Path(__file__).resolve().parents[1] / "dist"
worker = (dist / "_worker.js").read_text()
download_list = re.search(r"const DOWNLOAD_FILES = \[(.*?)\];", worker, re.S)
assert download_list, "Missing worker download allowlist"
download_files = set(re.findall(r'"([^"]+)"', download_list.group(1)))
pages = {p.relative_to(dist).as_posix(): Page(p) for p in dist.rglob("*.html")}
assert pages, "Build the site before checking it"
checked = 0
for filename, page in pages.items():
    # The five original studies remain a separate comparison archive.
    if filename.startswith("directions/"):
        continue
    if filename == "index.html":
        assert page.design == "nocturne", "Homepage must use Nocturne"
        assert "noindex" not in getattr(page, "robots", ""), "Homepage must be indexable"
        assert not any(href.startswith("/designs/") for href in page.links), "Remove preview navigation from homepage"
        assert "/docs/storm-analysis/" in page.links, "Missing analysis navigation"
    elif not filename.startswith("designs/"):
        assert "site-signal" in page.classes, f"Missing Signal design: {filename}"
        assert page.palette == "carbon", f"Missing default Carbon palette: {filename}"
        assert "/docs/storm-analysis/" in page.links, f"Missing analysis navigation: {filename}"
    checked += 1
    for href in page.links:
        url = urlsplit(urljoin("https://preview.local/" + filename, href))
        if url.netloc != "preview.local" or url.scheme != "https":
            continue
        target = unquote(url.path).lstrip("/")
        if target.startswith("download-file/"):
            assert target.removeprefix("download-file/") in download_files, f"Unknown download on {filename}: {href}"
            continue
        if not target or target.endswith("/"):
            target += "index.html"
        elif not (dist / target).is_file() and (dist / target / "index.html").is_file():
            target += "/index.html"
        assert (dist / target).is_file(), f"Broken link on {filename}: {href}"
        if url.fragment and target in pages:
            assert unquote(url.fragment) in pages[target].ids, f"Broken anchor on {filename}: {href}"
    for source in page.media:
        url = urlsplit(urljoin("https://preview.local/" + filename, source))
        if url.scheme != "https" or url.netloc not in {"preview.local", "hookecho.io"}:
            continue
        assert (dist / unquote(url.path).lstrip("/")).is_file(), f"Missing media on {filename}: {source}"

for css in dist.rglob("*.css"):
    for source in re.findall(r'url\(["\']?(/(?:shots|showcase)/[^)"\']+)', css.read_text()):
        assert (dist / unquote(urlsplit(source).path).lstrip("/")).is_file(), f"Missing background media in {css.name}: {source}"

assert any(src.startswith("https://app.hookecho.io/?embed#goto=") for src in pages["index.html"].frames), "Homepage must embed the actual HookEcho radar"
print(f"Verified homepage design, local links, download routes, and media across {checked} pages.")
