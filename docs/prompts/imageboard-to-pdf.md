# Prompt: imageboard thread → PDF archiver (Python)

Copy everything below the line into a coding assistant.

---

Build a Python 3.11+ command-line tool, `thread2pdf`, that takes the URL of an
imageboard thread and produces a single self-contained PDF that preserves the
thread faithfully: every post, every image at the best resolution available, and
every link between posts as a working, clickable link inside the PDF.

## Usage

```
thread2pdf URL [-o OUT.pdf] [--cache-dir DIR] [--max-image-px N]
           [--no-full-images] [--include-video-thumbs] [--delay SECONDS]
           [--user-agent UA] [--cookies FILE] [-v]
```

- `URL` is a thread page, e.g. `https://boards.4chan.org/g/thread/123456`,
  a vichan/Lynxchan/Tinyboard page, or a local saved `.html` file.
- If `-o` is not given, name the file `<board>_<thread_id>_<slug-of-subject>.pdf`.

## Architecture

Keep these as separate modules with small, testable interfaces:

1. **`fetch`** — HTTP with `httpx` (or `requests`): one shared session,
   configurable User-Agent, optional cookies, retry with exponential backoff on
   429/5xx, a polite delay between requests (default 1 s), and a disk cache keyed
   by URL hash so a re-run downloads nothing it already has.
2. **`extract`** — turns a thread into a list of `Post` dataclasses. Use an
   adapter per site family, chosen from the URL / page markup:
   - **JSON API first** where one exists (4chan `a.4cdn.org/{board}/thread/{no}.json`,
     vichan `/{board}/res/{no}.json`, Lynxchan `/{board}/res/{no}.json`). It gives
     exact original filenames, dimensions, sizes and full-size URLs.
   - **HTML fallback** with BeautifulSoup + lxml for everything else, including
     saved local files.
   - A generic adapter that uses common selectors (`.post`, `.postContainer`,
     `blockquote`, `.fileThumb`, `a[href*="/src/"]`) so unknown boards still work.
3. **`media`** — downloads and prepares images.
4. **`render`** — builds HTML from the posts and converts it to PDF.
5. **`cli`** — argparse/typer front end and progress output (`rich` or `tqdm`).

`Post` should hold at least: `id`, `is_op`, `subject`, `name`, `tripcode`,
`poster_id`, `flag/country`, `timestamp` (timezone-aware), `body_html`
(cleaned), `files: list[Attachment]`, `replies_to: list[id]`,
`replied_by: list[id]` (backlinks, computed after parsing).

`Attachment`: `full_url`, `thumb_url`, `original_filename`, `width`, `height`,
`size_bytes`, `mime`, `spoiler`, `md5/sha`, and later `local_path`.

## Images — high resolution is the main requirement

- Always try the **full-size file** (the `/src/` link the thumbnail points to),
  never the thumbnail, when it exists. Fall back to the thumbnail only if the
  full file 404s or is removed, and mark that post's image as "thumbnail only".
- Spoilered images: download the real image, not the spoiler placeholder.
- Do **not** re-encode JPEGs or PNGs that already fit; embed the original bytes
  so there is no quality loss. Only convert when the PDF engine can't embed the
  format:
  - WebP/AVIF/HEIC → lossless PNG (or high-quality JPEG for photos > ~4 MP if
    `--max-image-px` asks for it).
  - Animated GIF / APNG → keep the first frame as a still and note
    "animated, N frames" under it.
  - Video (webm/mp4) → extract a frame with `ffmpeg` if available (otherwise use
    the board thumbnail), overlay a ▶ marker, and link the image to the original
    video URL.
- Lay images out at a sensible display size (fit page width, keep aspect
  ratio), but keep the **full pixel data embedded** so the reader can zoom in.
  Downscale only if the user sets `--max-image-px`.
- Each image is a link to its original URL, with a caption line:
  original filename, pixel dimensions, file size.
- Deduplicate by hash: the same file posted twice is stored once in the PDF.
- Download concurrently (bounded, e.g. 4 workers), still respecting the delay.

## Links between posts — the second main requirement

- Every post gets a named anchor (`id="p123456"`), which becomes a PDF named
  destination.
- Rewrite every in-thread quote link (`>>123456`, `href="#p123456"`,
  `href="/g/thread/1#p123456"`) to an internal link to `#p123456`, so clicking it
  jumps to that post inside the PDF.
- Quote links to posts **outside** the thread (`>>>/g/123`, other-thread links)
  stay external links to the live URL, styled differently and marked "(external)".
- Links to posts missing from the thread (deleted) render struck through as
  "(dead)".
- Under each post's header list its **backlinks** ("Replies: >>124 >>130"), each
  one an internal link.
- Normal URLs in post text become clickable external links.
- Add PDF **bookmarks/outline**: one entry for the OP and one per post
  (`No.123456 – first 40 chars of text`), plus an optional "Images" section.
- Optional: a title page with board, thread subject, URL, post count, image
  count, and the archive timestamp.

## Rendering

- Use **WeasyPrint** (HTML + CSS → PDF). It supports internal anchors, external
  links, bookmarks via `bookmark-level`/`bookmark-label` CSS, and embeds images
  at their native resolution. Keep a Playwright/Chromium `page.pdf()` backend as
  an optional alternative behind a `--engine` flag if that's simple to add.
- Produce clean print CSS: readable serif/sans font with good Unicode coverage
  (bundle or point to Noto Sans + Noto Color Emoji / CJK fallback), greentext
  (`>` lines) in green, spoiler text shown with a light grey background,
  `code`/`[code]` blocks in monospace, post header with name, trip, ID, flag,
  date, and number. Avoid page breaks inside a post header; allow long posts to
  break across pages.
- Sanitize post HTML (only allow the tags you need) so board markup can't break
  the layout or pull remote resources at render time — every image must come
  from the local cache.

## Robustness

- Handle 404/archived/pruned threads with a clear error; work from a saved HTML
  file offline.
- Never crash on one bad image: log it, insert a placeholder box with the
  original URL, and continue.
- Print a summary at the end: posts, images (full / thumbnail-only / failed),
  internal links resolved, dead links, output size.
- Respect the site: user-configurable delay, no parallel page fetches, honour
  `Retry-After`.

## Deliverables

1. Package layout with `pyproject.toml` (console script entry point
   `thread2pdf`), pinned minimum versions of the dependencies.
2. README covering install (including the system libraries WeasyPrint needs and
   optional `ffmpeg`), usage examples, and supported boards.
3. `pytest` tests that run **offline** using saved fixture pages/JSON:
   - quote-link rewriting (in-thread, cross-thread, dead),
   - backlink computation,
   - full-size URL selection over thumbnails,
   - format conversion decisions,
   - an end-to-end test that builds a PDF from fixtures and checks with `pypdf`
     that it contains the expected number of images, that embedded image pixel
     sizes equal the originals, that internal link annotations point to the right
     named destinations, and that the outline has one entry per post.
4. Type hints throughout, `ruff`-clean.

Ask me before choosing anything that changes this spec (e.g. dropping
WeasyPrint); otherwise make sensible choices and state them in the README.
