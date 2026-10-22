---
paths:
  - "**/*.{ts,tsx,mts,cts}"
---

# TypeScript conventions

When writing TypeScript code, follow the following conventions:
  1. Strictness:
    - Assume `strict: true`; don't loosen tsconfig or add `@ts-ignore` to get past an error. If a suppression is unavoidable, use `@ts-expect-error` with a reason
    - No `any`. Use `unknown` and narrow with type guards, `in`, `instanceof`, or a schema parse
    - Avoid non-null `!` and `as` casts; narrow instead. If one is unavoidable, add a comment saying why it's safe
    - Assume `noUncheckedIndexedAccess`: handle `undefined` from `arr[i]` and `record[key]`
  2. Types:
    - Let inference type locals and internal helpers; annotate parameters and the return types of exported functions
    - Discriminated unions over boolean flags and optional-field soup (`{ kind: "ok", value } | { kind: "err", error }`, not `{ ok: boolean, value?, error? }`)
    - Exhaustive `switch` on discriminants, with a `never` check in `default`
    - Prefer `type` for unions and `interface` for object shapes meant to be extended; be consistent within a module
    - String-literal unions or `as const` objects over enums; never `const enum`
    - `satisfies` for typed config and literal objects; `as const` for constant tuples and tables
    - Branded types for IDs and validated values
  3. Error handling:
    - Validate external input (network, files, env, user input) at the boundary with the project's schema library (Zod, Valibot, ArkType) and derive types from the schema; trust types inside
    - Throw `Error` subclasses (with `cause` when wrapping) for unexpected failures; return a union for expected domain failures (not found, validation failed). Never throw strings or plain objects
    - No empty or swallowing `catch`. Rethrow with context, handle it, or log and say why it's safe to continue
    - `catch (e)` is `unknown`; narrow before using it
  4. Immutability:
    - `const` by default; `readonly` fields and `ReadonlyArray`/`readonly T[]` parameters unless the function owns the mutation
    - Return new objects and arrays rather than mutating arguments; use `toSorted`, `toReversed`, `toSpliced` over their mutating forms
  5. Iteration:
    - Prefer `map`, `filter`, `find`, `some`, `every`, and `flatMap` over index loops; use `for...of` when there are side effects or early exits
    - Avoid `reduce` when a clearer form exists; never `forEach` with `async` callbacks
  6. Async:
    - `async`/`await` over `.then()` chains
    - No floating promises: every promise is awaited, returned, or explicitly handed off with `void` and a comment saying why
    - Run independent work concurrently with `Promise.all` (all must succeed) or `Promise.allSettled` (partial failure is fine), not sequential `await`s in a loop
    - Long-running async functions accept an `AbortSignal`
  7. Modules:
    - Named exports; no default exports unless a framework requires them
    - `import type` for type-only imports
    - No namespaces; no barrel files inside an app (a package's single entry point is fine)

Check work with the project's typecheck (e.g. `tsc --noEmit`) rather than a full build, unless the user requests a build.

# Testing

Applies when writing TypeScript tests (e.g. as harness:test-agent). The project's `## Harness: testing layout` and `## Harness: quality gates` sections in `CLAUDE.md` take precedence over these defaults.

- Use the test runner the project already uses; follow its existing file placement (`*.test.ts` beside the source, or a `tests/` directory).
- Arrange / Act / Assert, one behaviour per test.
- Mock only the direct dependencies of the unit under test, injected through parameters or constructors. Avoid module-level mocking of deep imports.
- Test through exported APIs. Don't export internals or reach into private members to make a test possible; flag it instead.
- Keep test code type-checked: no `any` or `as` casts to build fixtures; write typed fixture builders.
- Skip a test blocked by a known bug with the runner's skip marker and a reason naming the bug: `it.skip("blocked by #<bug>: <scenario>", ...)`.
- For property-based tests, use `fast-check` if the project already depends on it.
- When the project exports library types, test them with type-level assertions (e.g. `expectTypeOf`).
- Name tests after behaviour: `"rejects an empty password"`, not `"test validate 3"`.
