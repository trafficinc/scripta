# Temporary Scripta serve improvement plan

This is a temporary implementation and review checklist for work performed in
Codex. Improve the existing standalone HTTP application server while retaining
isolated worker processes and deployment behind an HTTPS reverse proxy.

## Progress and approval rules

Current position: **Sections 1–3 accepted; Section 4 complete and validated, awaiting acceptance and explicit OK to begin Section 5.**

| Section | Scope | Status |
| --- | --- | --- |
| 1 | Reliable configuration | Accepted |
| 2 | Trusted reverse proxies | Accepted |
| 3 | Logs and health checks | Accepted |
| 4 | Deployment and shutdown | Complete; awaiting acceptance |
| 5 | Worker resilience | Not started |
| 6 | Development workflow | Not started |
| 7 | Performance and platform evidence | Not started |

For each section:

1. Obtain the user's explicit OK before starting implementation. Creating this
   file does not approve Section 1 or any later section.
2. Read the relevant implementation, tests, documentation, and `AGENTS.md`.
3. Implement the section with focused regression coverage, reference updates,
   and executable examples where applicable.
4. Run the required checks and record actual commands, results, and limitations
   in that section's evidence entry below.
5. Present the completed section for review. Stop at its approval gate; do not
   begin the next section until the user explicitly approves proceeding.

After a completed section is presented, an OK to proceed accepts that section
and authorizes the next named section. Clarify genuinely ambiguous approvals.
After Section 7, obtain final acceptance. Remove or archive this temporary file
only when directed by the user. Do not commit, publish, or release without
separate authorization. Preserve unrelated working-tree changes.

## Existing behavior and scope boundaries

- Preserve persistent worker processes, fresh per-request runtimes, bounded
  queues, execution deadlines, worker replacement, streaming/backpressure,
  request-owned upload cleanup, cookies, and sessions.
- Extend the existing bounded shutdown rather than treating it as absent.
- Keep Caddy, Nginx, or Apache responsible for public HTTPS in this milestone.
- No Apache module, FastCGI implementation, direct server-side TLS, automatic
  certificate management, Unix-domain listener, or WebSocket implementation is
  included. Each needs separate approval if added later.
- Routing, templates, authentication/authorization flows, CSRF policy, and new
  public cryptographic primitives are separate application/library work.
- Keep standard-library backends out of `scr-runtime`. Preserve minimum Rust
  compatibility, strict types, capability checks, resource ceilings, and the
  workspace's prohibition on unsafe code.
- Do not promise zero downtime, performance parity, or platform support without
  evidence. A graceful restart may still interrupt work after its drain deadline.

## Section 1 — Reliable configuration

- [x] Explicit approval to begin recorded: user replied “ok” after this plan was created.
- [x] Inventory current serve flags, defaults, fixed limits, and configuration
      patterns before choosing a file format or introducing dependencies.
- [x] Add an optional server configuration file with documented precedence:
      explicit CLI settings override file settings, which override defaults.
      Define interaction with existing `SCR_*` runtime settings explicitly.
- [x] Validate unknown keys, types, bounds, conflicting settings, and paths
      before accepting requests; report actionable errors without secrets.
- [x] Expose configurable connection lifetime, header-read timeout, and shutdown
      drain timeout with bounded, documented defaults and interactions with the
      request deadline.
- [x] Provide configuration validation and effective-setting inspection without
      opening a listener or exposing credentials/session contents.
- [x] Preserve existing CLI invocations and default-feature behavior.
- [x] Correct stale hosting documentation that still lists streaming responses
      as unsupported.
- [x] Test precedence, invalid configurations, boundary values, and backward
      compatibility; execute a configuration-based local server example.
- [x] Run checks, record evidence, and present for acceptance before Section 2.

Completion criteria: invalid configuration fails before listening; effective
settings can be inspected safely; existing serve commands continue to work.

Implemented contract:

- Optional flat TOML settings use the existing CLI TOML/Serde dependencies; no
  dependency or lockfile change. CLI settings override file settings regardless
  of option order. Relative file storage paths resolve beside the config file;
  CLI paths and the entry script retain working-directory resolution.
- `--check-config` and `--print-config` validate without listening, starting
  workers, running scripts, or creating storage. Inspection hides storage paths
  and reports recognized effective `SCR_*` runtime limits separately. File
  syntax/type errors do not echo configuration excerpts or supplied values.
- Connection lifetime, header-read timeout, and one shared shutdown drain budget
  are configurable in milliseconds, 1–3600000. Defaults are 60000, 10000, and
  11000 respectively. The drain budget is independent of the request timeout;
  it replaces the previous separate connection/worker drain waits.
- Added read-only host configuration validation and reusable session-limit
  validation. Storage is prepared/rechecked at actual startup; inspection does
  not guarantee write access, port availability, or successful application I/O.
- Updated CLI help, hosting/architecture/development docs, and the executed
  `examples/serve.toml` example. Corrected the stale streaming-support statement.

Validation evidence (2026-11-15):

- Base revision `44a6b5f` plus these working-tree changes. Native host macOS
  27.0.1 arm64; pinned Rust 1.99.1 host/target `aarch64-apple-darwin`;
  `MACOSX_DEPLOYMENT_TARGET=15.0`. No commit, release, or service installation.
- `cargo test -p scr-cli --test serve_config --test web_server --locked
  --offline --target aarch64-apple-darwin`: 45 tests passed. Coverage includes
  precedence, file-relative paths, redacted inspection while the configured port
  is occupied, absence of storage/application side effects, malformed/unknown/
  oversized configuration, types/bounds, symlinks/privacy, the shipped config
  serving real HTTP, CLI limit overrides, and connection/header/drain timeouts.
- The initial sandboxed run passed three configuration tests but could not bind
  the occupied-port fixture (`Operation not permitted`). Reran using the
  environment approval mechanism; focused and full tests passed with localhost
  and subprocess access. Assertions were retained.
- `cargo test --workspace --all-features --locked --offline --target
  aarch64-apple-darwin`: 587 passed, zero failures/ignored.
- `cargo test --workspace --locked --offline --target aarch64-apple-darwin`:
  587 passed, zero failures/ignored.
- `cargo test -p scr-cli --test serve_config --test web_server
  --no-default-features --locked --offline --target aarch64-apple-darwin`:
  44 passed, including configuration-based HTTP without SQLite.
- `cargo clippy --workspace --all-targets --all-features --locked --offline
  --target aarch64-apple-darwin -- -D warnings` and
  `cargo fmt --all -- --check` passed.
- `cargo +1.85.0 check -p scr-cli --locked --offline --target
  aarch64-apple-darwin` passed, including web/stdlib/runtime dependencies. This
  was a compile check, not a full minimum-Rust test run.
- Local Markdown reference paths and `git diff --check` passed.
- Linux/Windows and native macOS 15 baseline execution were not run. The
  deployment setting alone does not establish macOS 15 runtime validation.
  Trusted-proxy work remains unstarted pending Section 2 approval.

Acceptance / OK to proceed: user accepted with “ok”, approving Section 2.

## Section 2 — Trusted reverse proxies

- [x] Explicit approval to begin recorded: user replied “ok” after Section 1 was presented.
- [x] Define trusted proxy IP/CIDR configuration, defaulting to no trusted proxies.
- [x] Specify supported forwarded-header formats, precedence, malformed-input
      handling, and multi-proxy chain processing before implementation.
- [x] Resolve client address, external scheme, and host only through the defined
      trust policy; preserve the actual socket peer address separately.
- [x] Add explicit allowed-host validation and document Host/port/IPv6 rules.
- [x] Expose resolved request metadata through strict native APIs; update export
      references if public APIs change.
- [x] Keep session/cookie security defaults explicit; forwarded headers must not
      silently weaken cookie policy.
- [x] Test spoofed headers, untrusted peers, trusted chains, malformed/duplicate
      headers, host rejection, IPv4/IPv6, redirects, and direct local access.
- [x] Execute a controlled proxy deployment example. Use connected/local tools;
      install external software only through the environment's approval mechanism
      when needed.
- [x] Run checks, record evidence, and present for acceptance before Section 3.

Completion criteria: untrusted clients cannot forge resolved identity/scheme;
configured proxy deployments expose correct external request metadata.

Implemented contract:

- `trusted-proxies` and `allowed-hosts` accept bounded TOML string arrays or CLI
  comma lists, with no implicit proxy trust. CLI values replace file lists;
  an empty CLI value clears a list. No new dependencies or lockfile changes.
- The supervisor compiles literal-IP/CIDR trust and authority allowlists once.
  It walks one bounded X-Forwarded-For chain right to left to the first untrusted
  hop, and accepts scalar X-Forwarded-Proto/Host only from a trusted socket peer.
  Duplicate/malformed supported headers from trusted peers return 400 before
  body reception/application execution. Untrusted assertions do not affect
  resolution. Other forwarded-header families are ignored for resolution.
- Resolved host validation uses exact normalized names, bracketed IPv6, optional
  explicit ports, and no wildcard/suffix matching. Nonempty allowlists reject
  missing/disallowed external hosts. Host syntax remains validated independently
  of forwarded-host; HTTP/1.1 still requires Host.
- Added read-only `WebRequest.peerAddress`, `.clientAddress`, `.scheme`, and
  `.host`, transported as typed metadata through worker IPC. Raw headers are
  unchanged; Secure cookie/session defaults remain independent of scheme.
  Existing registered function counts remain unchanged.
- Rust request embeddings now provide `Request.connection: ConnectionInfo`;
  `Default::default()` leaves metadata unknown. Dynamic metadata strings count
  toward native request budgets; empty/unknown metadata preserves existing exact
  payload-budget behavior. Updated embedding/API/hosting/development references.
- Added formatted/executed `examples/web-proxy.scripta`, its
  `proxy-serve.toml` settings, and a reviewable `Caddyfile.proxy` configuration.
  No services were installed or public deployments performed.

Validation evidence (2026-11-15):

- Base revision `44a6b5f` plus working-tree changes from Sections 1–2. Native host
  macOS 27.0.1 arm64; Rust 1.99.1 host/target `aarch64-apple-darwin`, with
  `MACOSX_DEPLOYMENT_TARGET=15.0`. No commit, tag, or release was created.
- `cargo test -p scr-web --lib proxy --locked --offline --target
  aarch64-apple-darwin`: six tests passed for IPv4/IPv6 CIDRs and family boundaries,
  exact/over-limit entry and chain counts, authority normalization/ports, trusted
  chains, untrusted/unsupported header assertions, duplicates/malformed fields,
  and resolved-host allowlists.
- `cargo test -p scr-stdlib --test web --test web_sessions --test web_uploads
  --locked --offline --target aarch64-apple-darwin`: 30 passed, including
  host-supplied/unknown/read-only metadata, metadata budgets, failed assignment,
  existing exact cookie-result budgets, and unchanged session/upload behavior.
- `cargo test -p scr-cli --test serve_config --test web_server --locked
  --offline --target aarch64-apple-darwin`: 51 passed, with approved localhost/
  subprocess access. Tests cover file/CLI list replacement and clearing, invalid
  configuration without value disclosure, direct spoofing, IPC metadata,
  trusted chains, rejection before application execution, redirects, unchanged
  Secure cookies under forwarded http, and real IPv6 loopback.
- The shipped proxy script/configuration ran through a controlled sanitizing
  local HTTP forwarding fixture with forged client assertions. Separate Scripta
  syntax and formatter checks passed. Caddy/NGINX were not installed here; the
  fixture verifies the backend contract, not Caddy execution or public TLS.
- Full `cargo test --workspace` runs with default and `--all-features`, using
  `--locked --offline --target aarch64-apple-darwin`, each passed 600 tests with
  zero failures/ignored.
- The configuration/web suites with `--no-default-features` passed 50 tests,
  including the shipped proxy example without SQLite.
- Workspace all-target/all-feature Clippy with warnings denied, Rust formatting,
  local Markdown reference paths, and `git diff --check` passed.
- `cargo +1.85.0 check -p scr-cli --locked --offline --target
  aarch64-apple-darwin` passed, including web/stdlib/runtime dependencies; this was
  compilation, not a full minimum-Rust test run.
- Linux/Windows and native macOS 15 baseline execution remain pending. Public
  certificates and real multi-proxy deployments were not validated. Proxy
  operators must sanitize scalar scheme/host assertions and configure each trust
  boundary correctly. Section 3 remains unstarted pending approval.

Acceptance / OK to proceed: user accepted with “ok”, approving Section 3.

## Section 3 — Logs and health checks

- [x] Explicit approval to begin recorded: user replied “ok” after Section 2 was presented.
- [x] Add optional structured JSON logs while preserving a useful local format.
- [x] Generate bounded request IDs and correlate supervisor/worker diagnostics;
      define whether any caller-supplied ID is accepted and under what trust policy.
- [x] Record response completion separately from header readiness, including
      streamed failures, disconnects, bytes, and duration where measurable.
- [x] Add bounded worker/queue/timeout/failure counters without per-user or
      unbounded-label cardinality.
- [x] Provide optional private liveness/readiness endpoints with no application
      route collisions; define startup, overload, and draining semantics.
- [x] Keep credentials, cookies, session IDs, bodies, and query strings out of
      default logs; bound and escape untrusted log fields.
- [x] Test log validity/redaction, request correlation, streamed completion and
      failure records, counters, and health endpoint access/state behavior.
- [x] Run a controlled server example, validate, and present before Section 4.

Completion criteria: operators can distinguish overload, slow execution, worker
failure, and interrupted streams; health checks do not disclose application data.

Implemented contract:

- Added `--log-format text|json` and optional loopback-only
  `--health-address IP:port|off`, also available in TOML. Text remains the default;
  private management is disabled by default. Reused existing dependencies.
- Host-generated bounded request IDs use a process/startup prefix and a monotonic
  counter that cannot wrap. Incoming IDs are ignored, including through trusted
  peers. `WebRequest.requestId`, response `X-Request-ID`, supervisor events, and
  worker diagnostics correlate; application headers cannot replace the host ID.
- Separate `response_headers` and `response_finished` records distinguish body
  completion, failure, and interruption with duration and body-byte handoff counts.
  They do not claim remote delivery. HEAD/bodyless HTTP completion can precede
  worker execution completion; later diagnostics retain the same request ID.
- Default host records omit request/source/error contents and paths. Worker
  diagnostics use bounded codes/categories and byte spans. Serve startup syntax
  failures no longer print source excerpts; ordinary `scripta check` still does.
  Raw worker stderr is not inherited into host logs. Explicit native application
  stderr logging is forwarded as escaped, correlated events, at most 16 records
  per request retaining 4096 input bytes each. Existing application-log redaction
  and explicit file destinations remain available; deliberately logged content
  is still the application's responsibility.
- RAII request/body/queue/worker tracking and fixed saturating counters avoid
  retained request histories and dynamic metric labels. Snapshots include
  worker/queue capacity, body outcomes/bytes, overload, deduplicated request
  deadlines, connection errors/timeouts, and replacement activity.
- Separate private GET/HEAD `/livez`, `/readyz`, and `/metrics` endpoints never
  execute scripts or reserve application routes/slots. Their listener has eight
  connection slots, two-second deadlines, no keep-alive, and bounded headers.
  Readiness covers startup, shutdown, known worker leases, and full admission
  queues. Management stays available during application draining.
- Added/executed `examples/observe-serve.toml` with the existing proxy script,
  using ephemeral fixture ports. Updated API/host/architecture/development and
  embedding references; native function export counts remain unchanged.

Validation evidence (2026-11-15):

- Base revision `44a6b5f` plus working-tree changes from Sections 1–3. Native
  macOS 27.0.1 arm64, pinned Rust 1.99.1 host/target `aarch64-apple-darwin`, with
  `MACOSX_DEPLOYMENT_TARGET=15.0`. No dependency/lockfile change, service install,
  commit, tag, or release.
- `cargo test -p scr-web --lib --locked --offline --target
  aarch64-apple-darwin`: 13 passed, including ID exhaustion, exactly-once body
  accounting, counter saturation, startup/overload/drain readiness, RAII cleanup,
  stream error/drop/empty-body outcomes, and UTF-8-safe bounded log frames.
- Focused native `scr-stdlib --test web` coverage passed (12 tests); full runs
  also passed the expanded optional/read-only request-ID metadata assertions,
  exact existing value budgets, sessions, and uploads.
- `cargo test -p scr-cli --test serve_config --test web_server --locked
  --offline --target aarch64-apple-darwin`: 60 passed with approved localhost and
  subprocess access. Coverage includes safe config/startup errors, JSON validity,
  generated-ID headers/IPC, ignored caller/response IDs, default content privacy,
  streamed header versus body completion, post-commit errors, disconnects,
  deduplicated deadlines, queue overload/recovery, private route separation,
  GET/HEAD/method/body/query handling, and real termination-drain probes.
- The shipped observability configuration executed real application and private
  health/metrics requests; probe requests left application counters untouched.
  Explicit native application logging was escaped, correlated, and bounded.
- Full `cargo test --workspace` runs, default and `--all-features`, using
  `--locked --offline --target aarch64-apple-darwin`, each passed 613 tests with
  zero failures/ignored.
- The configuration/web suites with `--no-default-features` passed 59 tests,
  including observability/proxy examples without SQLite.
- All-target/all-feature workspace Clippy with warnings denied, Rust formatting,
  local Markdown reference paths, new-file whitespace checks, and
  `git diff --check` passed.
- `cargo +1.85.0 check -p scr-cli --locked --offline --target
  aarch64-apple-darwin` passed, including runtime/stdlib/web dependencies. This
  was compilation, not a full minimum-Rust test run.
- Linux/Windows and native macOS 15 baseline execution remain pending. Readiness
  reflects known handshake leases: idle child death can remain unknown until
  dispatch, and all-lost capacity still needs an application request for lazy
  recovery. Proactive supervision belongs to Section 5. Body handoff is not a
  browser receipt guarantee; a blocked host stderr sink can delay the supervisor.
  No public management authentication or external deployment was added.
  Section 4 remains unstarted pending approval.

Acceptance / OK to proceed: user replied “OK”, accepting Section 3 and approving Section 4.

## Section 4 — Deployment and shutdown

- [x] Explicit approval to begin recorded: user explicitly approved Section 4 — deployment and shutdown.
- [x] Make shutdown close idle keep-alive connections promptly and prevent new
      requests on existing connections while draining admitted work.
- [x] Apply the configured drain deadline consistently across connections,
      queued requests, workers, and streaming responses.
- [x] Preserve worker termination, session semantics, and spool cleanup when a
      drain expires; document that completed side effects cannot be rolled back.
- [x] Update readiness during shutdown and define repeated-signal behavior.
- [x] Document dedicated service accounts, persistent private data directories,
      service-manager startup/restart, and immutable application directories.
- [x] Provide reviewable Caddy/service configuration examples, with platform
      differences stated. Do not install services or alter host configuration
      merely to create these examples.
- [x] Document deployment by starting a replacement instance, checking readiness,
      switching the front end, and draining the old instance, including limitations.
- [x] Test idle/active keep-alive, queued work, long streams, deadline expiry,
      repeated signals, child cleanup, and a controlled service replacement.
- [x] Run checks, record evidence, and present for acceptance before Section 5.

Completion criteria: shutdown stops admission, drains within its configured bound,
and cleans up owned resources predictably; deployment instructions are executable.

Implemented contract:

- Per-connection watch notifications invoke Hyper graceful shutdown: idle
  keep-alive connections close promptly and active responses finish without
  accepting another request. Connections awaiting their first complete header
  close immediately on notification. Service-entry admission checks also reject
  requests buffered across shutdown, with a closing 503 if a response is sent.
  A partially received later header retains its header/drain ceilings.
- The application listener closes when the accept loop enters draining, within
  its existing 100 ms flag polling interval. One existing configured drain budget
  covers body reception, queues, workers, and streaming; existing earlier request
  and connection deadlines still apply. At expiry connections abort and worker
  loops receive a forced-stop notification, explicitly terminate/reap children,
  then release request-owned storage before temporary stores close. No new
  dependencies, public native APIs, or lockfile changes.
- Readiness uses the immediate shutdown flag; private liveness/metrics remain
  available through application cleanup. Repeated handled termination signals
  are idempotent, without forcing early exit or resetting the drain clock.
- Documented committed versus uncommitted sessions, persistent stores, spool
  ownership, irreversible completed side effects, cleanup overhead, dedicated
  accounts, immutable releases, service restart/stop policy, and platform gaps.
- Added reviewable Linux systemd template, blue/green TOML configurations, and
  Caddy example under `examples/deploy/`, with an operator sequence for readiness,
  application probes, front-end switch, old-instance drain, and rollback limits.
  Updated hosting, architecture, and development references.

Validation evidence (2026-11-15):

- Base revision `7561328` plus Section 4 working-tree changes. Native macOS
  27.0.1 arm64; pinned Rust 1.99.1 compiler host/target
  `aarch64-apple-darwin`; `MACOSX_DEPLOYMENT_TARGET=15.0` on Cargo build/test/lint
  commands. No commit, tag, release, service installation, or public deployment.
- `cargo test -p scr-cli --test web_server --test serve_config --locked
  --offline --target aarch64-apple-darwin`: 66 passed (60 web, six config).
  Six new regressions cover idle keep-alive/initial partial headers, active
  keep-alive and pipelining, admitted queued work, repeated signals, stream
  truncation, child termination/reaping, complete/partial upload spool cleanup,
  discarded queued side effects, persistent saved sessions, uncommitted-session
  rollback, controlled replacement, and the shipped deployment configurations.
- Initial sandboxed shutdown tests failed because localhost binding was blocked
  (`Operation not permitted`). Reran through the environment approval mechanism;
  retained all assertions. New fixture errors were corrected before final runs.
  A real Hyper initial-partial-header graceful-shutdown wait was found and fixed
  by closing connections which have never entered the request service.
- Controlled replacement ran two real Scripta instances with distinct listeners:
  verified replacement readiness, forwarded requests through a local front-end
  fixture before/after the target switch, drained an old active stream, and
  verified replacement service afterward. The shipped blue/green configurations
  served real HTTP with CLI overrides for ephemeral ports and private fixture
  storage; their proxy host allowlists and private health probes were exercised.
- `cargo test --workspace --all-features --locked --offline --target
  aarch64-apple-darwin` and `cargo test --workspace --locked --offline --target
  aarch64-apple-darwin`: each passed 619 tests, zero failures/ignored.
- `cargo test -p scr-cli --test serve_config --test web_server
  --no-default-features --locked --offline --target aarch64-apple-darwin`:
  65 passed, including shutdown, replacement, and deployment-config coverage.
- `cargo clippy --workspace --all-targets --all-features --locked --offline
  --target aarch64-apple-darwin -- -D warnings` and
  `cargo fmt --all -- --check` passed.
- `cargo +1.85.0 check -p scr-cli --locked --offline --target
  aarch64-apple-darwin` passed; compile coverage only, not minimum-Rust tests.
- Local Markdown reference paths, deployment-template whitespace, and
  `git diff --check` passed. Full test/lint command logs were retained locally
  under `/private/tmp/scripta-section4-*.log` for this session.
- Caddy and `systemd-analyze` are unavailable on this host. Their templates were
  reviewed against official references linked in the hosting guide, but Caddy
  execution/config validation, systemd installation/start/stop, launchd, Windows
  console/service wrappers, public TLS, and native Linux/Windows/macOS 15 baseline
  validation remain pending. No platform-support or zero-downtime claim was added.
  The drain bounds admitted work; OS process reaping, filesystem cleanup, and
  scheduling add overhead. A blocked host log sink remains a known limitation.
  External forced kills/machine failure can leave persistent-directory debris.
  Section 5 remains unstarted pending explicit approval.

Acceptance / OK to proceed: pending.

## Section 5 — Worker resilience

- [ ] Explicit approval to begin recorded.
- [ ] Add configurable recycling after a bounded request count without killing
      an active successful request solely to recycle its worker.
- [ ] Supervise unexpected worker-loop termination and restore configured capacity.
- [ ] Add bounded replacement backoff and define recovery/reset behavior so
      persistent startup failures cannot cause a rapid restart loop.
- [ ] Reflect usable worker capacity and recovery state in readiness and logs.
- [ ] Preserve admission limits, request deadlines, fresh request state, descriptor
      ownership, and request-storage cleanup across recycling and replacement.
- [ ] Test crashes, malformed/truncated IPC, repeated startup failures, recycling,
      capacity recovery, and shutdown while replacement is pending.
- [ ] Run checks, record evidence, and present for acceptance before Section 6.

Completion criteria: failure does not silently shrink capacity or trigger an
unbounded restart loop; healthy workers recover without resource leaks.

Validation evidence: pending.

Acceptance / OK to proceed: pending.

## Section 6 — Development workflow

- [ ] Explicit approval to begin recorded.
- [ ] Add explicit opt-in development/watch mode with documented watch roots,
      exclusions, debounce, symlink behavior, and bounded watch resources.
- [ ] Reload entry scripts and includes through a coherent worker generation;
      do not let one request mix runtime state from different generations.
- [ ] Define failed-reload behavior, in-flight request draining, and detection of
      multi-file edits. Do not imply arbitrary filesystem edits are transactional.
- [ ] Show actionable local syntax/startup errors without exposing diagnostics
      to public HTTP clients by default.
- [ ] Ensure ordinary production mode retains its existing explicit deployment
      behavior and does not start a file watcher.
- [ ] Test entry/include edits, rapid edits, invalid source, generation isolation,
      cleanup, and watch shutdown; execute the local developer workflow.
- [ ] Run checks, record evidence, and present for acceptance before Section 7.

Completion criteria: developers can refresh entry scripts and includes reliably
without enabling watch behavior in ordinary deployments.

Validation evidence: pending.

Acceptance / OK to proceed: pending.

## Section 7 — Performance and platform evidence

- [ ] Explicit approval to begin recorded.
- [ ] Define reproducible local workloads and bounded benchmark durations before
      measuring; record source revision/working-tree state and host/toolchain.
- [ ] Measure small JSON, SQLite, password hashing, binary upload/download,
      streaming, slow clients, and overload/recovery with controlled fixtures.
- [ ] Record throughput, latency distributions, memory observations, queue/worker
      behavior, and cleanup/recovery results; distinguish OS memory from logical
      runtime value budgets.
- [ ] Run sustained-load tests for leaks, worker recycling, client disconnects,
      and repeated failures without depending on public HTTP endpoints.
- [ ] Optimize only demonstrated bottlenecks, preserving semantics and adding
      meaningful regressions; rerun affected measurements after changes.
- [ ] Run native validation on available supported hosts using explicit target
      triples and repository guidance. Mark unavailable platforms as pending.
- [ ] Report minimum-Rust and feature coverage accurately. Keep compatibility
      claims unchanged without the evidence required by release documentation.
- [ ] Publish reproducible commands/results in repository development docs;
      do not publish releases or external artifacts as part of this section.
- [ ] Present final results, limitations, and the completed milestone for acceptance.

Completion criteria: repeatable measurements and recovery evidence exist; platform
results clearly distinguish verified behavior from pending validation. Missing
native hosts do not justify invented success or weakened assertions.

Validation evidence: pending.

Acceptance / final OK: pending.

## Validation shared by all sections

- Start with affected crates/integration targets and meaningful boundary/failure
  tests. Reuse local HTTP fixtures and controlled subprocesses.
- For public API, dependency, feature-wiring, or cross-crate changes, run focused
  tests followed by formatting, all-target/all-feature workspace Clippy with
  warnings denied, workspace tests, and `git diff --check`, per `AGENTS.md`.
- Check default/minimal/all-feature behavior where affected. Preserve SQLite's
  default feature and intentional locked dependency resolution.
- Use the pinned toolchain, explicit native target, and on macOS
  `MACOSX_DEPLOYMENT_TARGET=15.0`. Record actual OS/compiler/target; this deployment
  setting alone does not establish native macOS 15 runtime validation.
- Execute changed Scripta examples with controlled input after syntax/format
  checks. Update API/export references and architecture/development/hosting docs.
- Use the environment's approval mechanism when sandbox restrictions block
  required loopback/process tests; do not weaken assertions to bypass them.
- Documentation-only edits generally need path/link checks and `git diff --check`,
  not a full Rust test run. Do not repeat passing checks without a new reason.
- No GitHub Actions workflows, commits, tags, releases, service installation, or
  public deployment are authorized by this checklist.

## Repository references

- [Repository guidance](../AGENTS.md)
- [Web hosting](web-servers.md)
- [Standard library](standard-library.md)
- [Architecture](architecture.md)
- [Development](development.md)
- [Extension interfaces](writing-extensions.md)
- [Releases](releases.md)
- [Release checklist](release-checklist.md)
