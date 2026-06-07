# Civistash

Daily archiver for the most popular CivitAI images, with full metadata sidecars.

---

## 1. Synopsis

Civistash is a single-binary CLI written in Rust that fetches the top *N* most
popular images from the CivitAI REST API, downloads each image to a
date-partitioned local directory, and writes a sidecar JSON file containing
the complete API response for that image. It supports two execution modes: a
one-shot CLI invocation for use with external schedulers (cron, systemd timer,
Docker restart, Kubernetes CronJob), and an embedded `--daemon` mode that
loops with a cooldown equal to the requested period. Deduplication is handled
by scanning the existing output directory for the CivitAI image ID — no
database, no manifest file, no out-of-band state. All operation is offline
after each cycle: the only network call is the API fetch and the image
download itself. The program is platform-agnostic and ships with a
multi-stage distroless Dockerfile and CI matrices for Linux, macOS, and
Windows.

---

## 2. Goal

The primary objective of Civistash is to provide a **reliable, autonomous
local mirror of CivitAI's daily popular-image feed** with full generation
metadata preserved alongside each image, and with zero operational burden
on the user beyond providing an API token and an output directory.

Specifically:

- Fetch the top *N* popular images from `GET /api/v1/images` sorted by
  `Most Reactions` over a configurable `period` (default `Day`).
- Filter out non-image media (`type=video`, `type=audio`).
- Skip any image whose CivitAI ID already exists on disk in any date
  directory.
- Download the image binary to a date-partitioned path.
- Write a sidecar `.json` file containing the full API response, enriched
  with `downloaded_at`, `local_path`, and `source_url` fields.
- Honour HTTP 429 and 5xx responses with exponential backoff (1s → 2s → 4s,
  max 3 retries) before failing the cycle.
- Log individual download failures and continue, never aborting the whole
  cycle for one bad image.
- Run as a one-shot CLI or a long-lived daemon with `--period` cooldown.
- Handle SIGINT and SIGTERM gracefully in daemon mode by finishing the
  current image, writing its sidecar, and exiting cleanly.
- Build and run on Linux, macOS, and Windows, plus a multi-stage distroless
  Docker image.

---

## 3. Tech Stack

All versions verified against `crates.io` as of project kickoff. Lockfile is
the source of truth at build time.

```toml
[package]
name         = "civistash"
version      = "0.1.0"
edition      = "2024"
rust-version = "1.88.0"
description  = "Archiver for popular CivitAI images with full metadata sidecars"
license      = "MIT"

[[bin]]
name = "civistash"
path = "src/main.rs"

[dependencies]
# ── Command-Line Interface ─────────────────────────────────
clap         = { version = "4", features = ["derive", "env"] }

# ── Logging ────────────────────────────────────────────────
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

# ── Terminal Output ────────────────────────────────────────
owo-colors   = { version = "4", features = ["supports-colors"] }
indicatif    = "0.18"

# ── HTTP Client ────────────────────────────────────────────
reqwest      = { version = "0.13", default-features = false, features = ["rustls-tls", "json", "stream"] }
futures      = "0.3"

# ── Serialization ──────────────────────────────────────────
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"

# ── Time ───────────────────────────────────────────────────
chrono       = { version = "0.4", default-features = false, features = ["clock", "serde"] }

# ── Error Handling ─────────────────────────────────────────
anyhow       = "1"
thiserror    = "2"

# ── Release Profile ────────────────────────────────────────
[profile.release]
opt-level     = 3
lto           = "fat"
codegen-units = 1
strip         = true
panic         = "abort"
```

**Why these choices:**

- `clap` with `derive` + `env` matches the user's existing tool conventions
  (STNX) and gives us `env = "CIVITAI_TOKEN"` ergonomics for free.
- `owo-colors` with `supports-colors` matches STNX and is the de facto Rust
  crate for terminal styling without pulling in `crossterm` or
  `termcolor`.
- `reqwest` with `rustls-tls` (no OpenSSL) makes the binary statically
  portable across distros and musl.
- `chrono` with minimal features keeps the dependency tree lean — we only
  need local clock formatting and ISO 8601 serialisation.
- `panic = "abort"` in release pairs with `strip = true` for a small
  binary. The distroless image runs without a C library to catch panics.

---

## 4. Architecture Overview

Civistash is a single binary with a clear linear flow. There is no plugin
system, no runtime configuration reloading, no IPC. The architecture is a
strict pipeline:

```
                    ┌─────────────┐
   CLI flags ──────▶│ CLI parser  │
   ENV vars ───────▶│  (clap)     │
                    └──────┬──────┘
                           │
                           ▼
                    ┌─────────────┐         ┌──────────────┐
                    │  Bootstrap  │────────▶│ tracing init │
                    │             │         │  owo-colors  │
                    └──────┬──────┘         │  palette     │
                           │                └──────────────┘
                           ▼
                    ┌─────────────┐
                    │ reqwest     │  shared, reused across cycles
                    │ Client      │
                    └──────┬──────┘
                           │
                           ▼
                  ┌────────────────┐
                  │ Mode dispatch  │
                  └────┬───────┬───┘
                       │       │
              one-shot │       │ daemon
                       │       │
                       ▼       ▼
              ┌─────────────┐  ┌──────────────────────┐
              │  Fetch      │  │  Daemon loop         │
              │  cycle      │  │  ┌────────────────┐  │
              └──────┬──────┘  │  │  Fetch cycle   │  │
                     │         │  └────────┬───────┘  │
                     │         │           ▼          │
                     │         │  ┌────────────────┐  │
                     │         │  │ sleep(period)  │  │
                     │         │  │  + signal race │  │
                     │         │  └────────┬───────┘  │
                     │         │           │          │
                     │         │           ▼          │
                     │         │  ┌────────────────┐  │
                     │         │  │ shutdown?      │  │
                     │         │  │  (SIGINT/TERM) │  │
                     │         │  └────────────────┘  │
                     │         └──────────────────────┘
                     ▼
              ┌─────────────────────────────┐
              │ Fetch cycle                 │
              │ ┌─────────────────────────┐ │
              │ │ 1. GET /images          │ │
              │ │    (backoff on 429/5xx) │ │
              │ └────────┬────────────────┘ │
              │          ▼                  │
              │ ┌─────────────────────────┐ │
              │ │ 2. for each image:      │ │
              │ │    a. filter type       │ │
              │ │    b. dir-scan dedup    │ │
              │ │    c. download binary   │ │
              │ │    d. write sidecar     │ │
              │ │    e. log progress      │ │
              │ └─────────────────────────┘ │
              │ ┌─────────────────────────┐ │
              │ │ 3. print summary        │ │
              │ └─────────────────────────┘ │
              └─────────────────────────────┘
```

A **fetch cycle** is the atomic unit of work. The CLI runs exactly one
fetch cycle and exits. The daemon runs fetch cycles back-to-back,
interspersed with sleeps equal to the configured `--period` value, and
races each sleep against shutdown signals.

---

## 5. Key Components

### 5.1 `src/main.rs` — entry point and command dispatch

Responsibilities:

- Parse `Cli` via `clap::Parser`.
- Initialise the `tracing` subscriber with env-filter (`RUST_LOG`).
- Construct the shared `reqwest::Client` (with rustls, user-agent
  `civistash/0.1.0`).
- Dispatch to one-shot or daemon path.
- Set process exit code.

### 5.2 `src/cli/mod.rs` — argument definitions

Single `Cli` struct with no subcommands. The only operational difference
between one-shot and daemon mode is the `--daemon` flag. This keeps the
surface area small and the `clap` derive clean.

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "civistash", author, version, about, long_about = None)]
pub struct Cli {
    /// Run continuously, sleeping between cycles (cooldown = --period)
    #[arg(long, default_value_t = false)]
    pub daemon: bool,

    /// CivitAI API token (also via CIVITAI_TOKEN env)
    #[arg(short, long, env = "CIVITAI_TOKEN")]
    pub token: Option<String>,

    /// Output directory
    #[arg(long, default_value = "stash")]
    pub output_dir: PathBuf,

    /// Number of images to fetch per cycle
    #[arg(long, default_value_t = 100)]
    pub limit: u32,

    /// Time window: Day | Week | Month | AllTime
    #[arg(long, value_enum, default_value_t = Period::Day)]
    pub period: Period,

    /// Sort order
    #[arg(long, value_enum, default_value_t = SortOrder::MostReactions)]
    pub sort: SortOrder,

    /// NSFW level filter (omit to use account default)
    #[arg(long, value_enum)]
    pub nsfw_level: Option<NsfwLevel>,

    /// Log verbosity
    #[arg(long, default_value = "info")]
    pub log_level: String,
}
```

Supporting enums: `Period`, `SortOrder`, `NsfwLevel`. Each implements
`clap::ValueEnum` and `Display` for URL serialisation.

### 5.3 `src/api/mod.rs` — CivitAI REST client

Responsibilities:

- Build the request URL with query parameters.
- Apply bearer token if `Cli::token` is `Some`.
- Retry on 429 and 5xx with exponential backoff (1s → 2s → 4s).
- Parse the response into typed structures: `ImagesResponse { items,
  metadata }` and `Image` (the per-image object).
- Surface typed errors via `thiserror`.

Key public functions:

```rust
pub async fn fetch_popular(
    client: &reqwest::Client,
    token: Option<&str>,
    period: Period,
    sort: SortOrder,
    nsfw_level: Option<NsfwLevel>,
    limit: u32,
) -> Result<Vec<Image>, ApiError>;
```

The function does **not** paginate. The CivitAI API supports `limit` up
to 200 per call, and the user's `--limit` defaults to 100, so a single
request suffices. If a future requirement pushes the limit above 200, this
function would grow a `next_cursor` loop — that is intentionally out of
scope today.

### 5.4 `src/download/mod.rs` — image binary downloader

Responsibilities:

- For each `Image`, attempt to fetch the binary at `image.url`.
- Write to `{output_dir}/{date}/{id}.{ext}.partial` first, then rename to
  the final path on success. This guarantees atomic writes.
- Resolve the file extension from the URL path (e.g. `.jpeg`, `.png`,
  `.webp`).
- Honour the same exponential-backoff rules as the API client for the
  image download itself.
- Return `DownloadOutcome::Ok(path) | Skipped(AlreadyExists) |
  Failed(reason)` so the caller can log and decide.

```rust
pub async fn download_image(
    client: &reqwest::Client,
    image: &Image,
    dest_dir: &Path,
) -> DownloadOutcome;
```

### 5.5 `src/storage/mod.rs` — filesystem, dedup, sidecar writer

Responsibilities:

- Provide `today_partition_dir(base, date) -> PathBuf` returning
  `base/YYYY-MM-DD/`.
- Provide `id_exists_anywhere(base, id) -> bool` by walking all
  immediate subdirectories of `base` and checking for any file whose
  stem starts with the CivitAI image ID. This is the entire deduplication
  mechanism — no manifest, no DB.
- Provide `write_sidecar(image, local_path, source_url, dest_dir) ->
  Result<PathBuf, StorageError>` that serialises the enriched JSON and
  writes it to `{id}.json` next to the image.

```rust
pub fn id_exists_anywhere(base: &Path, id: i64) -> bool;
pub fn today_partition_dir(base: &Path) -> PathBuf;
pub fn write_sidecar(
    base: &Path,
    date: &NaiveDate,
    image: &Image,
) -> Result<PathBuf, StorageError>;
```

The `id_exists_anywhere` scan is O(N) over all subdirectories of the
output base, but in practice a year of daily runs yields ~36,500 files
across ~365 subdirectories, which is a single-digit-millisecond scan on
any modern filesystem. If this ever becomes a bottleneck, the right
optimisation is a per-date in-memory cache, not a database.

### 5.6 `src/daemon/mod.rs` — scheduler loop and signal handling

Responsibilities:

- Run the fetch cycle once.
- Sleep for the period (Day → 24h, Week → 168h, Month → 730h,
  AllTime → behaviourally a no-op cooldown; the daemon should not loop
  on `AllTime`).
- Race the sleep against SIGINT and SIGTERM.
- On signal, complete the current cycle's remaining work, then exit 0.

```rust
pub async fn run_daemon(
    client: &reqwest::Client,
    cli: &Cli,
) -> Result<(), DaemonError>;
```

Signal handling uses `tokio::signal::ctrl_c()` for SIGINT and, on Unix
only, `tokio::signal::unix::signal(SignalKind::terminate())` for
SIGTERM. On Windows, Docker sends a CTRL_BREAK_EVENT that tokio maps
onto `ctrl_c()`, so the same code path covers both platforms in
practice.

The cooldown is a `Duration` derived from the `Period` enum:

```rust
impl Period {
    pub fn cooldown(&self) -> Option<Duration> {
        match self {
            Period::Day => Some(Duration::from_secs(86_400)),
            Period::Week => Some(Duration::from_secs(604_800)),
            Period::Month => Some(Duration::from_secs(2_592_000)),
            Period::AllTime => None, // nonsensical to loop
        }
    }
}
```

### 5.7 `src/ui/mod.rs` — terminal theme and symbols

Responsibilities:

- Expose a small set of helpers wrapping `owo-colors::OwoColorize` that
  encode the "terminal mono + ember accent" palette (see §10.4).
- Expose the symbol constants used in the progress log.
- Provide `print_cycle_summary(stats: CycleStats)` that prints the final
  per-cycle totals with a `■` separator.

### 5.8 Module map

```
src/
├── main.rs        # entry, command dispatch
├── cli/
│   └── mod.rs     # clap definitions + Period/SortOrder/NsfwLevel enums
├── api/
│   └── mod.rs     # CivitAI REST client + retry/backoff
├── download/
│   └── mod.rs     # image binary downloader
├── storage/
│   └── mod.rs     # dir-scan dedup + sidecar writer
├── daemon/
│   └── mod.rs     # scheduler loop + signal handling
└── ui/
    └── mod.rs     # owo-colors theme + symbols
```

This is a flat single-crate layout. It is not a Cargo workspace because
the program is small, has no internal reusable component that benefits
from independent versioning, and the user prefers the simple
single-`Cargo.toml` convention.

---

## 6. Data Flow

### 6.1 Per-cycle flow

```
┌────────────────────┐
│ 1. Build request   │
│    GET /images     │
│    ?sort=...       │
│    &period=...     │
│    &limit=100      │
│    &browsingLevel= │ (only if --nsfw-level set)
│    [auth header]   │ (only if --token set)
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ 2. Send + retry    │
│    exp backoff     │◀──── 429 / 5xx
│    1s, 2s, 4s      │
│    max 3 attempts  │
└────────┬───────────┘
         │
         ▼
┌────────────────────┐
│ 3. Parse response  │
│    Vec<Image>      │
└────────┬───────────┘
         │
         ▼
┌────────────────────────────┐
│ 4. For each Image:         │
│    a. if type != "image"   │────▶ ⊘ filter (log, skip)
│    b. if id exists in dir  │────▶ ⦿ skip   (log, skip)
│    c. download binary      │──┐
│       with .partial file   │  │ 429/5xx/timeout
│    d. atomic rename        │  │──▶ ✗ failed (log, continue)
│    e. write sidecar JSON   │  │
│    f. progress: ✓ + ids    │◀─┘
└────────┬───────────────────┘
         │
         ▼
┌────────────────────┐
│ 5. Summary         │
│    downloaded: 87  │
│    skipped:   12   │
│    filtered:  1    │
│    failed:    0    │
│    duration: 14.2s │
└────────────────────┘
```

### 6.2 Storage layout

```
<output-dir>/
├── 2025-06-07/
│   ├── 12345.jpeg
│   ├── 12345.json
│   ├── 12346.png
│   ├── 12346.json
│   ├── 12350.webp
│   └── 12350.json
├── 2025-06-08/
│   └── ...
└── 2025-06-09/
    └── ...
```

The date directory name is `YYYY-MM-DD` in the local timezone, computed
from `chrono::Local::now().date_naive()`. Each image lives at
`{date}/{id}.{ext}` and its sidecar at `{date}/{id}.json`.

### 6.3 Sidecar JSON structure

The sidecar is the raw API response for that image, plus three enrichment
fields. All API fields are preserved verbatim; nothing is dropped or
restructured.

```json
{
  "id": 12345,
  "url": "https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/abc/original=true/abc.jpeg",
  "hash": "LKO2?U%2Tw=w]~RBVZRi};RPxuwH",
  "width": 1024,
  "height": 1024,
  "type": "image",
  "nsfw": false,
  "nsfwLevel": "None",
  "browsingLevel": 1,
  "createdAt": "2025-06-07T08:14:32.000Z",
  "postId": 67890,
  "username": "creator_handle",
  "baseModel": "SDXL 1.0",
  "modelVersionIds": [123, 456],
  "stats": {
    "cryCount": 0,
    "laughCount": 0,
    "likeCount": 1234,
    "dislikeCount": 0,
    "heartCount": 567,
    "commentCount": 89
  },
  "meta": null,

  "downloaded_at": "2025-06-07T14:22:33.456789Z",
  "local_path": "2025-06-07/12345.jpeg",
  "source_url": "https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/abc/original=true/abc.jpeg"
}
```

The `meta` object is only populated when the original uploader attached
generation metadata and the API request included `withMeta=true`. See
Open Question §8.2 for whether the default request should include it.

### 6.4 Request URL construction

```
GET https://civitai.com/api/v1/images
    ?sort=Most%20Reactions
    &period=Day
    &limit=100
    [&browsingLevel=<bitmask>]   # only if --nsfw-level is set
    [&withMeta=true]             # see Open Question §8.2

Authorization: Bearer <token>     # only if --token / CIVITAI_TOKEN is set
User-Agent: civistash/0.1.0
```

`browsingLevel` is passed as the integer bitmask; the mapping from
`NsfwLevel` enum variant to bitmask is an Open Question (§8.1).

---

## 7. Out of Scope

The following are explicitly **not** part of Civistash v0.1.0. Listing
them prevents scope creep and false expectations.

- **No database, no manifest, no index file.** Deduplication is
  directory scanning only.
- **No config file.** Token comes from `CIVITAI_TOKEN` env or `--token`
  flag. All other configuration is CLI flags.
- **No OAuth flow.** API keys only.
- **No model downloads.** Only `/api/v1/images`. The
  `/api/v1/models` and `/api/download/models/...` endpoints are out of
  scope.
- **No web UI, no HTTP server.** CLI only.
- **No automatic retry of failed individual downloads across cycles.**
  A failed download is logged and the cycle continues; on the next
  cycle, the image is not re-fetched from the API (its ID isn't in the
  failed list) but also doesn't appear again unless it bubbles up in
  the next popularity ranking. This is acceptable for v0.1.0.
- **No pagination of `/images` beyond `limit=200`.** Default of 100
  is well within the single-request cap.
- **No proxy, no cache, no rate-limit pacing beyond reactive backoff.**
  CivitAI does not publish rate-limit headers, so proactive pacing is
  guesswork. Backoff on observed 429s is the only mechanism.
- **No `?query=<text>` full-text search support.** Only the
  period/sort/limit trifecta.
- **No image transcoding, thumbnail generation, or format conversion.**
  Whatever CivitAI serves is what gets stored.
- **No deduplication by perceptual/content hash.** Dedup is by CivitAI
  image ID only.
- **No upload, no modification of CivitAI state.** Pure read-only
  consumer.
- **No support for `?period=AllTime` in daemon mode.** The cooldown
  for `AllTime` is undefined; the daemon exits with a usage error if
  `--daemon --period AllTime` is passed.
- **No Windows-specific signal handling beyond what tokio provides.**
  `tokio::signal::ctrl_c()` covers Docker-on-Windows in practice.

---

## 8. Open Questions

These are items where the user has not given a definitive answer and
where the developer (Builder) should confirm before implementation, or
pick a sensible default and flag it.

### 8.1 `browsingLevel` bitmask values

CivitAI's `browsingLevel` parameter is a bitmask, but the official docs
do not document the bit positions. Community SDKs suggest something like:

| Level   | Bit (suspected) |
|---------|-----------------|
| `None`  | 1               |
| `Soft`  | 2               |
| `Mature`| 4               |
| `X`     | 8               |

When `--nsfw-level` is set, the matching bit OR'd with `1` (to include
SFW) should be passed. **Builder to verify against the live API before
shipping.**

### 8.2 `?withMeta=true` default

The `meta` object contains the generation prompt, negative prompt, seed,
sampler, etc. It can be a few KB per image. For a 100-image cycle that
is ~100–500 KB of JSON, which is negligible. **Recommendation: always
send `?withMeta=true` by default.** The user can opt out with a future
`--no-meta` flag if they ever care.

### 8.3 File extension inference

The CDN URL ends in `.jpeg`, `.png`, or `.webp`. Three approaches:

1. **Parse the URL path** (recommended): `url.rsplit('.').next()`.
   Simple, no network round-trip.
2. **Inspect the `Content-Type` response header**: more accurate, but
   requires an HTTP HEAD or a download just to get the header.
3. **Sniff the magic bytes after download**: most accurate, but
   requires a re-read of the file.

**Recommendation: parse the URL path**, with a fallback to `.bin` if no
extension is present. The CivitAI CDN is consistent about extensions.

### 8.4 `.partial` file cleanup on shutdown

When a download is interrupted (daemon shutdown, Ctrl+C during one-shot,
network drop), the `.partial` file is left on disk. Three options:

1. **Leave them.** Next cycle's dedup check ignores them (it looks for
   `{id}.{ext}` without the `.partial` suffix), but they accumulate and
   waste disk space.
2. **Delete on shutdown.** Cleanup at the end of a graceful shutdown.
3. **Resume on next cycle.** If a `.partial` file exists for an ID,
   resume from its byte length with a `Range:` header.

**Recommendation: option 2** — delete `.partial` files in the relevant
date directory on graceful shutdown. Resuming (option 3) is a
non-trivial complication and the user did not ask for it.

### 8.5 Per-image retry budget

When an individual image download fails, do we retry it within the same
cycle, or log-and-skip?

**Recommendation: log-and-skip within the cycle.** The API-level retry
budget (3 attempts with backoff) is already applied to the image fetch.
Re-trying inside the cycle would slow down a 100-image run for one bad
URL. If the image is popular enough, it will appear in a future cycle
naturally.

### 8.6 Daemon sleep alignment

When `--daemon --period Day` is used, the daemon runs the first cycle
immediately, then sleeps 24 hours, then runs again. Should the second
cycle align to a wall-clock boundary (e.g. always at 03:00 local), or
just 24 hours after the first cycle started?

**Recommendation: 24 hours after the previous cycle's start time.**
This is simpler and the user did not request wall-clock alignment. If
they want it later, a `--align-to <HH:MM>` flag can be added.

---

## 9. Success Criteria

The project is "done and working correctly" when **all** of the
following hold:

1. `cargo build --release` produces a single `civistash` binary on
   Linux, macOS, and Windows, with no external C library dependencies.
2. `civistash --token <valid_token>` exits 0, creates a date
   subdirectory under `./stash/`, downloads *N* images, and writes *N*
   sidecar JSON files alongside them.
3. Running `civistash --token <valid_token>` a second time immediately
   after the first: every image is skipped, no network calls are made
   for the image binary, and the exit code is 0.
4. `civistash --token <valid_token> --limit 5` produces exactly 5
   downloaded images.
5. `civistash --daemon --period Day --token <valid_token>` runs the
   first cycle, then prints a "sleeping" message, then waits. A
   `kill -TERM <pid>` (or Docker stop) finishes the current image (if
   any) and exits cleanly with code 0 within a few seconds.
6. `civistash` with no token exits non-zero with a clear, ember-styled
   error message.
7. `cargo clippy --all-targets --all-features -- -D warnings` passes
   with zero warnings.
8. `cargo fmt --all --check` passes with the configured style
   (hard tabs, tab_spaces = 4).
9. The Docker image builds via `docker build -t civistash/civistash .`
   and runs the binary as a non-root user with the distroless base.
10. CI on GitHub Actions passes the `Lint`, `Linux Build`, `macOS
    Build`, and `Windows Build` workflows for every push to `main` and
    every pull request targeting `main`.

---

## 10. Reference Snippets

### 10.1 `Cargo.toml`

See §3 above for the full file.

### 10.2 `justfile`

Exact copy of the STNX justfile with the project name updated in the
header comment.

```makefile
# civistash build orchestration
# Install `just` via: cargo install just

# Use PowerShell on Windows, sh on Unix
set windows-shell := ["pwsh.exe", "-NoLogo", "-Command"]
set shell := ["bash", "-uc"]

# Default recipe - build everything
default: dev

# Verify formatting before build-related tasks
fmt:
    cargo fmt --all --check

# Development build with debug symbols and no optimizations, also run the linter
dev: fmt lint
    cargo build

# Release build with all optimizations
release: fmt lint
    cargo build --release --locked

# Clean all build artifacts
clean:
    cargo clean

# Run cargo check
check: fmt lint
    cargo check

# Run cargo clippy linter
lint: fmt
    cargo clippy --all-targets --all-features -- -D warnings

# Run tests
test: fmt lint
    cargo test
```

### 10.3 `rustfmt.toml`

Exact copy of the STNX rustfmt.toml.

```toml
# `rustup component add rustfmt` to install rustfmt if you don't have it already.

hard_tabs = true

tab_spaces = 4
```

### 10.4 Source style guide (terminal mono + ember accent)

**Palette.** All output is monospace. The default terminal foreground is
the base colour; everything else is a deliberate emphasis.

| Role                       | Colour         | Example use                                |
|----------------------------|----------------|--------------------------------------------|
| Default (base)             | terminal fg    | plain text                                 |
| Ember (primary emphasis)   | `#d97757`      | timestamps, paths, the app name            |
| Forest (success)           | `#5a8a5a`      | `✓` marks, "complete" lines                |
| Brick (error)              | `#c14545`      | `✗` marks, error messages                  |
| Dim (secondary)            | `#7a7a8c`      | filtered/skipped items, "idle" markers     |
| White emphasis             | bold terminal  | counts, byte sizes                         |

Implementation in `src/ui/mod.rs` using `owo-colors`:

```rust
use owo_colors::{OwoColorize, Style, Rgb};

pub const EMBER:  Style = Style::new().fg::<Rgb(217, 119, 87)>();
pub const FOREST: Style = Style::new().fg::<Rgb( 90, 138,  90)>();
pub const BRICK:  Style = Style::new().fg::<Rgb(193,  69,  69)>();
pub const DIM:    Style = Style::new().fg::<Rgb(122, 122, 140)>();

pub const SYM_OK:        &str = "✓";
pub const SYM_FAIL:      &str = "✗";
pub const SYM_SKIP:      &str = "⦿";
pub const SYM_FILTER:    &str = "⊘";
pub const SYM_PROGRESS:  &str = "⤓";
pub const SYM_IDLE:      &str = "…";
pub const SYM_RETRY:     &str = "↻";
pub const SYM_DIVIDER:   &str = "■";
```

**Status reporting rules.**

- `eprintln!` for status messages during a cycle.
- `println!` for the final summary only.
- Every status line begins with a symbol, a space, then a styled
  message.
- Counts and byte sizes in the summary line are bold terminal default.

Example progress lines (representative; the actual colour codes are
applied at runtime, not in the string literals):

```
⤓  Downloading 12345.jpeg   (1024×1024, 1.4 MB)
✓  Downloaded  12345.jpeg   → stash/2025-06-07/12345.jpeg
⦿  Skipped     12346.jpeg   (already present)
⊘  Filtered    12347.mp4    (type=video)
✗  Failed      12348.jpeg   (HTTP 404)
…  Sleeping 24h until next cycle
↻  Retrying GET /images (attempt 2/3) after 2s
```

**Error reporting rules.**

- All errors are wrapped in `anyhow::Error` at the call site and
  formatted with the brick colour and a leading `✗` symbol.
- Error messages are one line, no stack traces in normal operation.
  The `RUST_LOG=civistash=trace` env var exposes the tracing chain for
  debugging without changing the user-facing output.

### 10.5 `LICENSE`

Exact copy of the STNX license with the project name updated in the
copyright line if desired. (The user may keep the personal name
"Hyphonical" or change it — this is a project-level decision.)

```
MIT License

Copyright (c) 2026 Hyphonical

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### 10.6 `.gitignore`

```gitignore
# Generated by Cargo
# will have compiled files and executables
debug
target

# These are backup files generated by rustfmt
**/*.rs.bk

# MSVC Windows builds of rustc generate these, which store debugging information
*.pdb

# Generated by cargo mutants
# Contains mutation testing data
**/mutants.out*/

# RustRover
#  JetBrains specific template is maintained in a separate JetBrains.gitignore that can
#  be found at https://github.com/github/gitignore/blob/main/Global/JetBrains.gitignore
#  and can be added to the global gitignore or merged into this file.  For a more nuclear
#  option (not recommended) you can uncomment the following to ignore the entire idea folder.
#.idea/

# .env file
.env

# Local civistash output (downloaded images and sidecars)
stash/
```

### 10.7 `Dockerfile`

Multi-stage build. Stage 1 uses the official Rust image to compile a
fully static, stripped binary. Stage 2 uses Google's distroless
`cc-debian12:nonroot` base for a minimal attack surface and small image
size. The binary runs as the `nonroot` user (UID 65532).

```dockerfile
# ── Stage 1: Build ────────────────────────────────────────
FROM rust:1.88-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Pre-fetch dependencies for better layer caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/civistash*

# Compile the real source
COPY src ./src
RUN touch src/main.rs \
    && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/civistash /usr/local/bin/civistash
COPY LICENSE /LICENSE

USER nonroot
WORKDIR /stash
VOLUME ["/stash"]

ENV CIVITAI_TOKEN=""
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/civistash"]
```

### 10.8 `.dockerignore`

```
target
.git
.gitignore
.stash
stash
**/*.partial
```

### 10.9 `.github/workflows/ci-lint.yml`

Exact copy of the STNX lint workflow with the project name replaced in
the clippy invocation (the lint command is identical because it runs
against the whole workspace).

```yaml
name: Lint

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

env:
  CARGO_TERM_COLOR: always

jobs:
  clippy:
    name: 🔍 Clippy
    runs-on: ubuntu-latest
    steps:
      - name: 📥 Checkout
        uses: actions/checkout@v4

      - name: 🦀 Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy

      - name: 📦 Cache Cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-lint-${{ hashFiles('**/Cargo.lock') }}

      - name: 📎 Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings
```

### 10.10 `.github/workflows/ci-linux.yml`

Exact copy of the STNX Linux workflow with the binary name `civistash`
substituted in for `stnx`, and the artifact name updated.

```yaml
name: Linux Build

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: 🚧 Linux x86_64
    runs-on: ubuntu-latest

    steps:
      - name: 📥 Checkout
        uses: actions/checkout@v4

      - name: 🦀 Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-unknown-linux-gnu

      - name: 📦 Cache Cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: linux-x86_64-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: 🚧 Build Release
        run: cargo build --release --target x86_64-unknown-linux-gnu

      - name: 📦 Stage Files
        run: |
          mkdir dist
          cp target/x86_64-unknown-linux-gnu/release/civistash dist/
          [ -f LICENSE ] && cp LICENSE dist/ || true
          [ -f README.md ] && cp README.md dist/ || true

      - name: 📤 Upload
        uses: actions/upload-artifact@v4
        with:
          name: civistash-linux-x64
          path: dist/*
          compression-level: 9
          if-no-files-found: error
          retention-days: 7
```

### 10.11 `.github/workflows/ci-macos.yml`

Exact copy of the STNX macOS workflow with the binary name `civistash`
substituted and the artifact name updated.

```yaml
name: macOS Build

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: 🚧 macOS aarch64
    runs-on: macos-latest

    steps:
      - name: 📥 Checkout
        uses: actions/checkout@v4

      - name: 🦀 Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin

      - name: 📦 Cache Cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: macos-aarch64-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: 🚧 Build Release
        run: cargo build --release --target aarch64-apple-darwin

      - name: 📦 Stage Files
        run: |
          mkdir dist
          cp target/aarch64-apple-darwin/release/civistash dist/
          [ -f LICENSE ] && cp LICENSE dist/ || true
          [ -f README.md ] && cp README.md dist/ || true

      - name: 📤 Upload
        uses: actions/upload-artifact@v4
        with:
          name: civistash-macos-aarch64
        path: dist/*
        compression-level: 9
        if-no-files-found: error
        retention-days: 7
```

### 10.12 `.github/workflows/ci-windows.yml`

Exact copy of the STNX Windows workflow with the binary name
`civistash` substituted and the artifact name updated.

```yaml
name: Windows Build

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: 🚧 Windows x86_64
    runs-on: windows-latest

    steps:
      - name: 📥 Checkout
        uses: actions/checkout@v4

      - name: 🦀 Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: x86_64-pc-windows-msvc

      - name: 📦 Cache Cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: windows-x86_64-cargo-${{ hashFiles('**/Cargo.lock') }}

      - name: 🚧 Build Release
        run: cargo build --release --target x86_64-pc-windows-msvc

      - name: 📦 Stage Files
        run: |
          mkdir dist
          copy target\x86_64-pc-windows-msvc\release\civistash.exe dist\
          if (Test-Path LICENSE) { copy LICENSE dist\ }
          if (Test-Path README.md) { copy README.md dist\ }

      - name: 📤 Upload
        uses: actions/upload-artifact@v4
        with:
          name: civistash-windows-x64
          path: dist/*
          compression-level: 9
          if-no-files-found: error
          retention-days: 7
```

### 10.13 `.github/dependabot.yml`

Exact copy of the STNX dependabot configuration.

```yaml
# To get started with Dependabot version updates, you'll need to specify which
# package ecosystems to update and where the package manifests are located.
# Please see the documentation for all configuration options:
# https://docs.github.com/code-security/dependabot/dependabot-version-updates/configuration-options-for-the-dependabot.yml-file

version: 2
updates:
  - package-ecosystem: "cargo" # See documentation for possible values
    directory: "/" # Location of package manifests
    schedule:
      interval: "daily"
```
