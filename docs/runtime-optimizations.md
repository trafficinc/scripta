# Runtime optimizations

Scripta keeps ordinary AST evaluation as the reference behavior. Bounded fast
paths handle common cases and fall back when their guards do not apply.

## Main paths

| Area | Implementation |
| --- | --- |
| Calls | [Numeric leaf calls](../crates/scr-runtime/src/numeric_call.rs), then [integer self-recursion](../crates/scr-runtime/src/prepared_call.rs), then ordinary evaluation. |
| Loops | [Integer](../crates/scr-runtime/src/integer_loop.rs), [float](../crates/scr-runtime/src/float_loop.rs) and [numeric-call](../crates/scr-runtime/src/numeric_call_loop.rs) plans for supported bodies. |
| Collections | [Copy validation](../crates/scr-runtime/src/copy_validation.rs), [map construction](../crates/scr-runtime/src/map_builder.rs) and checked [map positions](../crates/scr-runtime/src/map_index.rs). |
| Objects | Checked [method hints](../crates/scr-runtime/src/method_cache.rs) and [field positions](../crates/scr-runtime/src/objects.rs). |
| Storage | Small [binding stores](../crates/scr-runtime/src/bindings.rs), shared declarations, and bounded scope/argument reuse. |

Checked binary integer arithmetic is shared through
[`ops::integer`](../crates/scr-runtime/src/ops.rs). Specialized scalar storage stays
separate from public `Value`; changing representations requires measurement.

## Rules to preserve

- Preserve evaluation order, strict types, errors, source spans, traces and
  logical execution limits.
- Decline before executing a planned body. Once execution starts, propagate
  failures without replaying script effects. Assignment fallback reuses an
  already evaluated RHS; completed writes remain visible after later failures.
- Validate hints against current bindings, keys and callable identities.
  Names, spans and positions alone do not establish identity.
- Preserve collector roots, runtime ownership and cleanup across callbacks.
  Reused storage must be cleared and must not bypass resource accounting.

See the [language specification](language-spec.md) for semantics and
[architecture](architecture.md) for ownership and crate boundaries.

## Contributor checklist

1. Implement ordinary behavior first. Review affected call, loop, mutation and
   lookup paths when adding operators, types or AST forms.
2. Keep unsupported cases on ordinary evaluation. Use the public
   [native interface](writing-extensions.md) for extensions; effectful calls
   require ordinary call handling unless explicitly proven safe.
3. Test the changed behavior, fallback and failure boundaries. Keep path-hit
   assertions and independent expected results when implementations share code.
4. Follow [repository validation](../AGENTS.md#tests-and-validation). Benchmark
   production changes against matched binaries with checked outputs and control
   workloads; distinguish API execution from whole-process CLI time. See the
   [benchmark guide](../benchmarks/README.md).
