# Streaming Data Download and Single-Flight Installation

Status: approved design

Date: 2026-07-11

## Context

The current data path keeps the complete gzip response and the complete
decompressed SQLite database in two `Vec<u8>` values. With the published
`data-v1` artifact this means at least roughly 183 MiB plus 561 MiB of live
payload memory, excluding allocation capacity and library buffers.

The current 900-second `ureq::AgentBuilder::timeout_read` is an individual
socket-read timeout, not a deadline for the complete request. The response and
gzip output are also unbounded. Finally, first installation publishes through
the shared name `data.tmp`, so concurrent processes can write and rename the
same temporary file.

## Goals

- Keep transfer and decompression memory bounded by small fixed buffers.
- Enforce compressed and decompressed byte limits.
- Enforce connect, idle-read, total HTTP, and lock-wait timeouts.
- Allow only one data download per data directory at a time.
- Make first installation and explicit update use the same fully validated
  candidate-publish path.
- Keep an existing live database byte-for-byte unchanged on every failure
  before a successful atomic replacement.
- Preserve the existing CLI contract and data-v1 URL/checksum pinning.
- Work on Linux, macOS, and Windows with Rust 1.95.

## Non-goals

- Download resumption or range requests.
- Automatic retry or mirror selection.
- Remote data-version discovery.
- Data schema or data-v1 artifact changes.
- Changing normal query ranking, output, or offline behavior.
- Serializing ordinary readers behind a long-running download lock.

## Chosen Approach

Use a two-stage disk pipeline protected by a persistent OS file lock:

```text
HTTP response
  -> unique compressed sibling (incremental SHA-256 and compressed limit)
  -> MultiGzDecoder
  -> unique SQLite candidate (decompressed limit)
  -> file sync
  -> schema/version + quick-check + FTS integrity verification
  -> file sync
  -> atomic publish
```

This keeps memory O(buffer size) and verifies the authenticated compressed
artifact before spending work on decompression. It uses about 183 MiB more
temporary disk than a one-pass decoder, but has simpler and safer checksum,
trailing-data, and cleanup semantics.

## Components and Boundaries

### Transfer module

A focused private submodule under `data` owns:

- `DownloadPolicy`, including all byte and time limits;
- strict response-length handling;
- streaming HTTP reads, progress, compressed byte counting, and incremental
  SHA-256;
- streaming multi-member gzip decoding and decompressed byte counting;
- uniquely named temporary artifact creation and ordinary-failure cleanup.

It does not open SQLite, decide whether an install is needed, or publish a
candidate as the live database.

Production defaults are:

- connect timeout: 30 seconds;
- idle read timeout: 60 seconds;
- total HTTP timeout: 15 minutes;
- lock wait timeout: 20 minutes;
- compressed limit: 256 MiB;
- decompressed limit: 768 MiB;
- transfer buffer: 64 KiB.

Tests can inject smaller limits and timeouts without changing production
constants.

### Operation lock

The lock path is a permanent sibling named `data.sqlite.lock`. The
implementation opens it with `create(true)` and uses the Rust standard
library's OS file-lock API, available below the project's Rust 1.95 MSRV. No
new locking dependency is needed.

Lock acquisition uses `try_lock` with bounded polling rather than an
unbounded blocking call. It reports once that another fojin process is active,
then fails clearly after the lock-wait deadline. The lock file is never deleted:
process exit or crash releases the OS lock, so there is no stale-lock breaking
algorithm and no lock-file deletion race.

- `ensure_data` retains its unlocked existing-file fast path. If data is
  missing, it creates the parent directory, acquires the operation lock, and
  checks again before downloading.
- `update_data` acquires the same lock for its complete operation, preventing
  duplicate downloads and last-finisher version reversal.
- `data clean` acquires the same lock before deleting the live data and known
  temporary artifacts. It leaves the lock file in place.
- Ordinary queries do not acquire this lock and can keep using the previous
  live database while an update downloads and validates its candidate.

On Windows, a non-cooperating reader may still prevent final replacement. The
existing replacement recovery contract remains: a candidate is preserved only
when replacement entered an ambiguous recovery state, and the error reports
its exact path. The download lock is not extended to readers because doing so
would block queries for the full transfer duration.

### Artifact lifecycle

Compressed and decompressed artifacts are siblings of the live database so
the final rename stays on one filesystem. Names include the live filename,
artifact role, process ID, and an atomic sequence, and are opened with
`create_new(true)`:

- `data.sqlite.download.<pid>.<sequence>.gz`
- `data.sqlite.candidate.<pid>.<sequence>`

An ownership guard removes only artifacts created by the current operation.
It is disarmed only after a successful publish or an intentional Windows
preservation result. Recoverable errors never sweep another process's files.

## Detailed Data Flow

1. Acquire the operation lock and perform the mode-specific second check.
2. Create a unique read/write compressed artifact.
3. Build a ureq agent with connect, idle-read, and total HTTP timeouts. Send
   `Accept-Encoding: identity` so the bytes counted and hashed are the release
   asset bytes rather than transparent HTTP content decoding.
4. Treat `Content-Length` as an early rejection/progress hint, never as the
   resource limit. A missing length is allowed. When transfer encoding does
   not override a present value, reject non-numeric values, duplicates, and
   values above the compressed limit.
5. Read the response with a fixed buffer. Before every write, checked-add the
   cumulative count, enforce the compressed limit, update SHA-256, write the
   chunk, and emit decile progress when a trustworthy total is available.
6. At EOF, require any usable declared length to match the received count and
   compare the final digest with the pinned SHA-256. A mismatch deletes the
   compressed artifact before decompression starts.
7. Rewind the compressed file and stream a `MultiGzDecoder` into a unique
   candidate. Copy at most `max_uncompressed + 1` bytes so an exact-limit gzip
   still reads and verifies its trailer while an oversized stream is detected.
   Multi-member gzip is supported; truncation, invalid CRC/ISIZE, or trailing
   non-gzip data fails closed.
8. Flush, `sync_all`, and close the candidate. Run the existing complete
   schema/version, `PRAGMA quick_check`, and FTS content-integrity checks.
   First installation now receives the same validation as explicit update.
9. Sync the validated candidate again and publish with the existing
   platform-specific atomic replacement logic.
10. Remove the compressed artifact and any ordinary candidate sidecars.

No production transfer path returns the complete response or decompressed
database as a `Vec<u8>`. Existing public byte helpers may remain for source
compatibility and small unit tests, but the installer and updater do not call
them.

## Error and Recovery Contract

Errors distinguish these stages:

- operation-lock wait timeout or lock I/O failure;
- connection, HTTP deadline, idle read, status, or response read failure;
- invalid, duplicate, mismatched, or oversized response length;
- compressed stream limit exceeded;
- SHA-256 mismatch;
- gzip format, trailer, or decompressed stream limit failure;
- candidate creation, write, flush, or sync failure;
- SQLite compatibility, quick-check, or FTS integrity failure;
- final platform-specific replacement failure.

Network and checksum failures retain the existing manual-download guidance.
Every failure before successful publish satisfies both invariants:

1. an existing live database is unchanged; and
2. a first installation does not expose a live database path.

Ordinary failures remove both owned artifacts and SQLite sidecars. If cleanup
also fails, the cleanup error is attached as context without hiding the primary
failure. The intentionally preserved Windows recovery case is the only
exception, and it explicitly reports the validated candidate path.

## Testing Strategy

### Deterministic unit and local HTTP tests

- compressed size exactly at the limit and one byte over;
- decompressed size exactly at the limit and one byte over;
- known oversized `Content-Length` rejected before body consumption;
- missing length and chunked transfer constrained by the actual byte counter;
- invalid, duplicate, and mismatched declared lengths;
- valid digest and digest mismatch;
- single-member and multi-member gzip;
- truncated gzip, bad trailer, and trailing garbage;
- a small gzip that expands past the configured limit;
- a server that continuously dribbles bytes: idle timeout does not fire but
  the injected total HTTP deadline does;
- a body pause longer than the injected idle timeout;
- injected reader/writer failures without platform-specific `/dev/full`;
- every error leaves an existing live database unchanged and removes owned
  artifacts.

### Process-level concurrency tests

A test re-executes the Rust test binary as two worker processes against one
slow local HTTP server and one temporary data directory. It asserts:

- both callers succeed;
- the server observes exactly one request;
- the waiter reuses the installed database after the lock-time second check;
- the final SQLite and FTS verification succeeds;
- no owned compressed or candidate artifacts remain.

Additional tests serialize update versus update and clean versus install, and
verify that the lock is released after worker exit. Windows CI retains the
existing replacement recovery tests and exercises the new lock path.

### Regression suite

The acceptance run includes:

- Rust 1.95 formatting, Clippy with warnings denied, all locked tests, release
  build, and package verification;
- Python normalization parity;
- release and installer shell contracts;
- ShellCheck and actionlint when their covered files change.

## Documentation and Delivery

README documents the 256 MiB compressed limit, 768 MiB decompressed limit,
timeouts, single-flight behavior, temporary disk requirement, and unchanged
offline behavior. CHANGELOG records the bounded-memory transfer and concurrent
installation fix.

Delivery uses the established repository process: focused commits on an
isolated branch, independent code review, GitHub pull request, all checks green,
then merge to `master`. This work does not create a tag or GitHub Release.

## Acceptance Criteria

- The production download/update path has no full-response or full-database
  in-memory allocation.
- Compressed input above 256 MiB and decompressed output above 768 MiB fail
  before publish.
- A continuously active response cannot exceed the 15-minute HTTP deadline.
- Two concurrent first installations make one HTTP request and both succeed.
- First installation and update both perform complete SQLite/FTS validation.
- All injected failures preserve the live database and clean owned artifacts.
- Rust, Python, shell-contract, Windows, and GitHub CI checks pass.
