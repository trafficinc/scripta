# Development Guidelines

<!-- repogroom:section:general:start -->
Follow the repository's local conventions and instructions. Inspect relevant code
before changing it, verify claims against evidence, and report only checks and
tests actually performed. Preserve licenses, copyright notices, and attribution.

Profile and language supplements apply only where relevant. In mixed projects,
each language's rules apply to its own files and module boundaries. Command examples
are suggestions to check against local configuration and tool availability; they
are not claims that tools are installed or checks have passed. Prefer the project's
documented commands, and do not install tools or choose frameworks just to match
these examples. RepoGroom does not execute them.

## Code simplicity

Prefer clear, direct code with a small scope. Avoid speculative features, layers,
and abstractions. Remove duplication only when the shared behavior is established.

## Architecture

Respect existing boundaries and separate responsibilities where there is a concrete
need. Explain significant tradeoffs. Do not add services or frameworks without a
requirement that justifies them.

## Naming

Use precise names consistent with nearby code and language conventions. Prefer
familiar terms and make units and ownership clear when they matter.

## Meaningful comments

Explain intent, constraints, and non-obvious decisions. Keep useful comments and
directives. Avoid comments that merely repeat the code, and update comments when
the behavior they describe changes.

## Documentation

Document observable behavior, setup, and limitations accurately. Verify examples
and claims. Preserve custom documentation and legitimate attribution; do not add
unsupported promises or generated implementation reports.

## Tests

Test changed behavior and meaningful edge cases using the project's conventions.
Run relevant checks and state what passed, failed, or could not be run. Never
invent results, benchmarks, or coverage claims.

## Security

Validate untrusted input at boundaries, handle errors explicitly, and use least
privilege. Do not expose credentials or sensitive source in logs or reports.
Preserve existing security checks and avoid unsafe shell interpolation.

## Dependencies

Prefer existing capabilities and standard libraries. Justify new dependencies,
check their compatibility and licensing, and follow the local versioning policy.
Do not remove dependencies without verifying their uses and obtaining the review
required by the project.

