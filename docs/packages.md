# Packages

Package management is built into `scripta`. Git packages require the `git` executable
on PATH; local packages do not. There is no hosted registry in this version.

The runnable [local package example](../examples/packages/main.scripta) includes a
manifest, lockfile, and library. From `examples/packages`, run
`scripta install --locked --offline` followed by `scripta main.scripta`.

## Use a package

From your project directory:

```sh
scripta init
scripta add helpers --path ./lib/helpers
# Or install a Git package with semantic-version tags:
scripta add helpers --git https://example.com/team/helpers.git --version '^1.2'
# Or pin a branch, tag, or commit, resolved to an exact commit in the lockfile:
scripta add helpers --git https://example.com/team/helpers.git --rev v1.2.3
```

`init` creates `scripta.toml` and refuses to overwrite an existing manifest.
`add` records the dependency and installs the dependency graph. An existing name
is replaced, allowing you to change its source or version constraint. Failed
resolution leaves the previous manifest, lockfile, and installation intact.
Package commands other than `init` find the closest manifest in the current
directory or its ancestors. Paths supplied to `add` are relative to your current
directory. Paths saved in a manifest resolve against that manifest's directory;
`add` keeps paths inside the project relative and uses absolute paths for external
folders. For a portable project, edit external paths to suitable relative paths
before committing and run `scripta install` again. Local Git repository paths supplied
to `add` are made absolute. Windows drive and UNC paths are passed to Git using
forward slashes without extended-length prefixes, so local repositories are
not mistaken for SSH hostnames. Existing manifests containing canonical
Windows paths are normalized when fetched as well.

```scr
<?scr
require_once "helpers" as Helpers;
echo Helpers::greeting();
```

A bare dependency name resolves to its installed entry file. An existing local
file with that name takes precedence. Other local paths retain their existing
behavior. Packages must be declared as direct dependencies of the closest
`scripta.toml` to the including source file. Functions retain their definition
file, so later calls resolve dependencies relative to their defining package.
All script files still start with `<?scr`.

## Package aliases and isolation

Add `as Alias` to load a package into an isolated module scope:

```scr
<?scr
require_once "helpers" as Helpers;
require_once "other-tools" as Tools;
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

`require_once` caches successful isolated instances by canonical entry path per
runtime. Different aliases of the same file share one instance and its state;
separately installed dependency versions have different paths and stay isolated.
Every load still checks resolution, permissions, host policy, and package
integrity before using a cached instance. Cycles are checked before cache lookup.
`require` always executes in a fresh isolated scope. On success it binds the alias
and replaces the cached instance for subsequent `require_once` loads; existing
other aliases and callbacks retain their previous instance. Successful cached
instances remain alive until runtime drop and count toward scope limits.

Failed initialization publishes no new alias or cache entry and preserves any
previous binding. Runtime scope/source/loading state is restored. Output, file
changes, and successful dependency initialization that occurred earlier are not
rolled back. Failed loads can be retried. Native permissions, source/include/call
limits, and logical value budgets continue to apply; aliases are name isolation,
not a security sandbox.

The same syntax works for local files: `require_once "lib/helpers.scripta" as Helpers;`.
Without `as`, includes retain the existing shared-scope behavior and duplicate
function errors. Unaliased `require_once` tracking is separate for the application
and each isolated module, so a local helper file can be included once in each.
Aliased and application shared-scope loading have separate caches. An initialized
module entry is marked complete for unaliased `require_once` inside that module,
including later calls from its functions. Existing local-file precedence
over dependency names is unchanged.

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
create that source file yourself. For compatibility, existing package manifests
that omit `entry` still default to `main.scr`. Explicit `.scripta`, `.scta`, and
`.scr` entry paths are all accepted. Names start with a lowercase ASCII letter and
contain lowercase letters, digits, underscores, or hyphens (maximum 64 characters).
The dependency name must match the package's declared name. Package versions use
semantic versioning. Unknown manifest fields are rejected to catch typos.

Git dependencies require exactly one of `version` or `rev`. Version constraints
support forms such as `^1.2`, `~1.2`, `=1.2.3`, and `>=1.0, <2.0`. Resolution picks
the highest matching semantic-version tag, accepting both `1.2.3` and `v1.2.3`.
The package's declared version must satisfy the constraint. A `rev` dependency
may name a branch, tag, or commit; installation pins its resolved commit. Git
sources are repositories whose root contains `scripta.toml`. There is no monorepo
subdirectory selector, registry lookup, publish command, or install-time script.
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
