"""Run after npm run build: verify the shared design and local navigation."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids, self.links, self.classes, self.frames = set(), [], [], []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "html":
            self.classes = attrs.get("class", "").split()
            self.palette = attrs.get("data-palette")
        if tag == "a" and attrs.get("href"):
            self.links.append(attrs["href"])
        if tag == "iframe":
            self.frames.append(attrs.get("src", ""))


dist = Path(__file__).resolve().parents[1] / "dist"
pages = {p.relative_to(dist).as_posix(): Page(p) for p in dist.rglob("*.html")}
assert pages, "Build the site before checking it"
checked = 0
for filename, page in pages.items():
    # The five original studies remain a separate comparison archive.
    if filename.startswith("directions/"):
        continue
    assert "site-signal" in page.classes, f"Missing Signal design: {filename}"
    assert page.palette == "carbon", f"Missing default Carbon palette: {filename}"
    assert "/docs/storm-analysis/" in page.links, f"Missing analysis navigation: {filename}"
    checked += 1
    for href in page.links:
        url = urlsplit(urljoin("https://preview.local/" + filename, href))
        if url.netloc != "preview.local" or url.scheme != "https":
            continue
        target = unquote(url.path).lstrip("/")
        if not target or target.endswith("/"):
            target += "index.html"
        elif not (dist / target).is_file() and (dist / target / "index.html").is_file():
            target += "/index.html"
        assert (dist / target).is_file(), f"Broken link on {filename}: {href}"
        if url.fragment and target in pages:
            assert unquote(url.fragment) in pages[target].ids, f"Broken anchor on {filename}: {href}"

assert any(src.startswith("https://app.hookecho.io/?embed#goto=") for src in pages["index.html"].frames), "Homepage must embed the actual HookEcho radar"
print(f"Verified Signal design, analysis navigation, and local links across {checked} pages.")
