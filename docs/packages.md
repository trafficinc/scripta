# Packages

Package management is built into `scripta`. Git packages require the `git` executable
on PATH; local packages do not. Package lookup uses a configurable local or
Git-hosted index. No public registry or official package catalog is bundled.

The runnable [local package example](../examples/packages/main.scripta) includes a
manifest, lockfile, and library. From `examples/packages`, run
`scripta search`, `scripta add helpers --offline`, and
`scripta install --locked --offline`, followed by `scripta main.scripta`. Its index
and package source are local, so this example needs no network or Git.

## Use a package

From your project directory:

```sh
scripta init
# With a configured index:
scripta add scripta/template
# Or provide a source explicitly:
scripta add helpers --path ./lib/helpers
# Or install a Git package with semantic-version tags:
scripta add helpers --git https://example.com/team/helpers.git --version '^1.2'
# Or pin a branch, tag, or commit, resolved to an exact commit in the lockfile:
scripta add helpers --git https://example.com/team/helpers.git --rev v1.2.3
```

`init` creates `scripta.toml` and a `main.scripta` greeting when the source
path is absent. It keeps existing source unchanged and refuses to overwrite an
existing manifest.
`add` records the dependency and installs the dependency graph. An existing name
is replaced, allowing you to change its source or version constraint. Failed
resolution leaves the previous manifest, lockfile, and installation intact.
Project commands find the closest manifest in the current directory or its
ancestors. `search` can also run outside a project with `--index` or
`SCR_PACKAGE_INDEX`. Paths supplied to `add` are relative to your current
directory. Paths saved in a manifest resolve against that manifest's directory;
`add` keeps paths inside the project relative and uses absolute paths for external
folders. For a portable project, edit external paths to suitable relative paths
before committing and run `scripta install` again. Local Git repository paths supplied
to `add` are made absolute. Windows drive and UNC paths are passed to Git using
forward slashes without extended-length prefixes, so local repositories are
not mistaken for SSH hostnames. Existing manifests containing canonical
Windows paths are normalized when fetched as well.

```scr
require "helpers" as Helpers;
echo Helpers::greeting();
```

A bare dependency name resolves to its installed entry file. An existing local
file with that name takes precedence. Other local paths retain their existing
behavior. Packages must be declared as direct dependencies of the closest
`scripta.toml` to the including source file. Functions retain their definition
file, so later calls resolve dependencies relative to their defining package.

## Package lookup and search

Configure an index in `scripta.toml`:

```toml
[registry]
index = "./scripta-index.toml"
# Or: index = "https://example.com/team/scripta-index.git"
```

`--index <source>` overrides `SCR_PACKAGE_INDEX`, which overrides `[registry].index`.
Manifest index paths resolve against the manifest directory; command-line and
environment index paths resolve against the current directory. Local files and
directories containing `scripta-index.toml` are supported. Git repositories are
cloned into a temporary checkout; the index must be a regular file named
`scripta-index.toml` at the repository root. Git-hosted lookup is explicit and
uses Git's normal TLS verification and credential configuration.

A catalog maps package names to descriptions and sources:

```toml
format = 1

[packages."scripta/template"]
description = "HTML templates"
source = { git = "https://example.com/team/template.git", version = "^1.0" }

[packages."scripta/db"]
description = "Local database helpers"
source = { path = "./libraries/db", version = "^1.0" }
```

Each entry needs exactly one source, `path` or `git`. Local source paths resolve
against the index directory. Git-hosted indexes must use absolute Git URLs and
cannot reference local package paths. A Git source may specify either a version
constraint or a revision; omitting both uses `*` (the highest stable version tag).
Unknown fields, invalid names, incompatible source options, and malformed version
constraints are rejected. Index format 1 allows up to 4 MiB, 10,000 entries, and
4,096 UTF-8 bytes per description.

```sh
scripta search                   # list catalog entries
scripta search template          # case-insensitive name/description substring
scripta search --index ./catalog/scripta-index.toml --offline
scripta add scripta/template
scripta add scripta/template --version '^2.0'
scripta add scripta/db --offline  # local index and local package source
```

`add` saves the resolved path/Git source and constraint into `[dependencies]`,
then uses the existing installer to pin commits and checksums. `--version` or
`--rev` overrides the index entry's selection. The full package name must match
the dependency's `[package].name`; `owner/name` is a package identity, not an
implicit GitHub address. `search` never installs packages or changes project
files. Search result order is deterministic by package name.

`--offline` lookup requires a local index file/directory. Remote indexes are not
cached. Offline `add` can reuse existing valid Git snapshots, but cannot fetch a
new Git package. Explicit `--path` and `--git` sources bypass the index. Later
`install`, `update`, and script execution use saved sources and do not consult
an index; deleting the catalog does not break an installed package. A changed
catalog mapping takes effect only when you run `add` again. This provides lookup
without requiring a hosted registry or introducing a publish command.

## Package aliases and isolation

Add `as Alias` to load a package into an isolated module scope:

```scr
require "helpers" as Helpers;
require "other-tools" as Tools;
echo Helpers::greeting();
echo Tools::greeting();
```

Aliases are unquoted, case-sensitive identifiers. `Helpers::greeting()` uses the
same call notation as `String::trim()`. Each module has its own functions and
variables, so both packages can define `greeting` and same-named helper functions.
A module cannot read or overwrite caller variables or call caller script
functions. Registered native functions remain available. Module functions capture
their module's lexical state and call their own helpers normally. Functions
declared directly in the module scope, including through unaliased local
includes, are callable through the alias; there is no separate export/private
function declaration. Module variables are not exposed through the alias.

Aliases are lexical bindings, visible to nested functions and retained callbacks.
Aliases inside a module stay within that module. A visible alias cannot be rebound
to a different canonical entry file; this raises `ModuleAliasError` before running
that file. Registered namespaces such as `String`, `File`, and `Http` cannot be
used as aliases. Missing module functions raise `ModuleError`. Static checking
validates the syntax without loading or resolving modules.

Aliased `require` and `require_once` cache successful isolated instances by
canonical entry path per runtime. Different aliases of the same file share one
instance and its state;
separately installed dependency versions have different paths and stay isolated.
Every load still checks resolution, permissions, host policy, and package
integrity before using a cached instance. Cycles are checked before cache lookup.
Prefer `require "package" as Alias;` for new library code; aliased `require_once`
is a compatible spelling. Both initialize a module only once per runtime and bind
new aliases to the cached instance. Editing a loaded source file does not reload
it. No export declarations are needed: all direct module-scope functions are
callable through the alias. Successful cached instances remain alive until runtime drop and count toward scope limits.

Failed initialization publishes no new alias or cache entry and preserves any
previous binding. Runtime scope/source/loading state is restored. Output, file
changes, and successful dependency initialization that occurred earlier are not
rolled back. Failed loads can be retried. Native permissions, source/include/call
limits, and logical value budgets continue to apply; aliases are name isolation,
not a security sandbox.

The same syntax works for local files: `require "lib/helpers.scripta" as Helpers;`.
Without `as`, includes retain the existing shared-scope behavior and duplicate
function errors. Unaliased `require_once` tracking is separate for the application
and each isolated module, so a local helper file can be included once in each.
Aliased and application shared-scope loading have separate caches. An initialized
module entry is marked complete for unaliased `require_once` inside that module,
including later calls from its functions. Existing local-file precedence
over dependency names is unchanged.

Aliased `require` no longer provides fresh instances. Unaliased includes retain
their existing repeated-execution semantics and should be reserved for deliberate
shared-scope loading.

## Manifest

A runnable application only needs `[dependencies]`. A reusable package must also
provide `[package]`:

```toml
[package]
name = "helpers"
version = "1.2.3"
entry = "src/main.scripta"

[dependencies]
strings = { git = "https://example.com/team/strings.git", version = "^1.0" }
config = { path = "../config", version = "^2.0" }
```

`scripta init` declares `entry = "main.scripta"` explicitly for new projects;
the starter source is created when absent. For compatibility, existing package manifests
that omit `entry` still default to `main.scr`. Explicit `.scripta`, `.scta`, and
`.scr` entry paths are all accepted. Names start with a lowercase ASCII letter and
contain lowercase letters, digits, underscores, or hyphens. Both `name` and
`owner/name` are accepted, with a maximum of 64 characters per part. Quote names
containing `/` in TOML, for example `"scripta/template" = { path = "../template" }`.
The dependency name must match the package's declared name. Package versions use
semantic versioning. Unknown manifest fields are rejected to catch typos.

Git dependencies require exactly one of `version` or `rev`. Version constraints
support forms such as `^1.2`, `~1.2`, `=1.2.3`, and `>=1.0, <2.0`. Resolution picks
the highest matching semantic-version tag, accepting both `1.2.3` and `v1.2.3`.
The package's declared version must satisfy the constraint. A `rev` dependency
may name a branch, tag, or commit; installation pins its resolved commit. Git
sources are repositories whose root contains `scripta.toml`. There is no monorepo
subdirectory selector, publish command, or install-time script. Index lookup
resolves to ordinary Git/local dependencies before installation.
Private repositories use the installed Git client's credentials; interactive
terminal credential prompts are disabled.

Local dependencies may optionally specify a version constraint. Their contents
are copied into the installation, so editing the original does not affect an
already installed package. Run `scripta update` to accept those edits. Relative local
dependencies within a Git package must be available in that repository's checked
out tree. Git submodules are not fetched.

## Install and update

```sh
scripta install                   # reuse existing pins; resolve new/changed dependencies
scripta install --locked          # fail if the root manifest and lockfile differ
scripta install --locked --offline # use installed Git snapshots without network access
scripta update                    # resolve all dependencies again and accept source changes
scripta update --offline          # update local sources; Git resolution requires a network-capable install
```

Commit `scripta.toml` and `scripta.lock`. Ignore `.scripta/` and
`.scripta-install.lock`. The versioned lock format records each dependency source,
package version, exact Git commit when applicable, SHA-256 content checksum, and
its transitive dependency tree. Each package has a private installation of its
dependencies, allowing different packages to use different versions of the same
library. Identical dependencies are currently copied rather than deduplicated.

Installs stage the full graph before replacing `.scripta/packages`. Ordinary
installation failures preserve the previous tree and metadata. Installation is
serialized by a project lock file; after a forcibly terminated install, remove a
stale `.scripta-install.lock` only after confirming no installer is running.
There is no crash-recovery journal spanning the tree and metadata replacements.

Existing Git installations with matching checksums can be reused without fetching.
If a Git snapshot is missing or modified, an online install fetches its locked
commit and verifies its checksum. Offline installs fail if that snapshot cannot
be reused. Local sources are always checked against their pinned checksum; source
changes require `scripta update`. `--locked` requires an existing matching root
lockfile and also validates pinned transitive content.

Execution never downloads packages, invokes Git, or reads the original local
source. Package resolution validates installed contents against the project
lockfile and raises `PackageError` for stale metadata, missing installations,
or checksum mismatches. `scripta check` continues to check only the supplied source.
The runtime's read permission, include-depth limits, and module path policy
continue to apply to loading package entry scripts. Embedders opt in with
`Runtime::set_module_resolver`; `scr_cli::pipeline` configures the package resolver.
Custom resolvers are host code and must enforce any host policy on their own
metadata reads.

Package snapshots exclude `.git`, `.scripta`, and `.scripta-install.lock` entries.
All other files are included, including untracked files for local sources. File
modes and timestamps do not contribute to checksums. Symlinks, special files,
non-UTF-8 filenames, and filenames containing backslashes are rejected. Each
package is limited to 64 MiB of files, 10,000 files, and 64 directory levels;
dependency graphs are limited to 256 installations and 32 active package levels.
These are installer limits, not a sandbox for executing package code.

## Inspect and remove dependencies

```sh
scripta list                  # locked direct dependencies, versions, source kinds
scripta list --tree           # include private transitive dependencies
scripta outdated              # inspect direct dependency sources, without updates
scripta outdated --offline    # inspect local sources; report Git queries as skipped
scripta remove scripta/mysql  # remove a direct dependency and its private subtree
```

`list` reports the lockfile graph, rather than asserting that every installed file
is present or intact. `outdated` reports CURRENT, WANTED (the highest version
matching the saved constraint), and LATEST (the highest stable Git version tag).
It reports changes to moving Git branches/tags and local content, including local
changes that keep the same version. Immutable commit pins are intentional and
are not reported as outdated. Missing matching Git version tags are reported;
source failures return an error. Git inspection uses `ls-remote` and can require
network access; `--offline` marks those checks unknown instead of claiming that
they passed. Both commands require a matching lockfile when dependencies exist
and leave manifests, lockfiles, and installed snapshots unchanged. `outdated`
returns success for a completed report even when newer versions are available.

`remove` accepts direct dependencies only. It stages the retained graph from
verified installed snapshots, prunes the removed package and its private
dependencies, and atomically replaces project metadata using the existing
installation transaction. Retained packages keep their versions and checksums,
even if original local sources have changed or disappeared. A dependency still
needed by another package remains in that package's private installation. Missing
or corrupted retained snapshots cause failure without changing the project; run
`install` to repair them first. No network access or index is needed for removal.

## Test a package

Write native Scripta tests in `tests/*_test.scripta` and run `scripta test` after
installing dependencies. Optional `[test]` settings configure discovery directories,
per-case timeouts, and output limits; package commands preserve them. No testing
package or Python installation is required. See [the testing guide](testing.md).
