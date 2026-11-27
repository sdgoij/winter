# Proposal to Slag: a host resolver for static import specifiers

Status: implemented in Slag `d02fba3`, usable from a host since `48038e0`, and
used by Winter's loader. One engine gap remains: `Context::set_host_hooks` does
not forward the resolver (see "Implementation status").
Date: 2026-11-27.
Audience: the engine (`github.com/sdgoij/slag`, pinned at `48038e0`).
Related: ADR 086 and `docs/extensions-scope.md` in this repository; the
consumer is `crates/extensions` with its `host` feature.

## Implementation status

The resolver landed as proposed — `HostHooks::resolve_imported_specifier`, and
`host_resolve_imported_module` taking the referrer and keying the cache, the
registry and the parse's name by the answer — together with the three smaller
asks below, and with one addition: `HostHooks::resolve_imported_module`, for a
host that hands the engine a module record of its own rather than bytes.

The last blocker for an embedder was `JsString`, which the hook's signature
carries and which was not re-exported at the crate root; `48038e0` adds it.
`ModuleSourceKind` came in `f89960b`. Winter is pinned to `48038e0` and now
resolves `(specifier, referrer)` itself, registers modules under their root path,
and has deleted both the root-spelling rule and the ambiguity refusal.

One engine bug still shapes the host. `Context::set_host_hooks` wraps the host's
hooks in an internal `RejectionHooks` that forwards only
`ensure_can_compile_strings`, `promise_rejection_tracker` and `create_worker`
(`crates/runtime/src/embed.rs`). `resolve_imported_specifier` — and the other
hooks added since the wrapper was written — never reach a hook installed the
supported way, and the failure is silent: the resolver simply is not called.
Winter installs its hooks through `Context::agent_mut().host_hooks` instead, and
its hook implements `promise_rejection_tracker` so unhandled rejections still
reach the log.

| Engine gap | What it blocks | Fix |
|---|---|---|
| `RejectionHooks` in `embed.rs` forwards only three methods | `set_host_hooks(Box::new(host))` silently drops the resolver, the host module resolver, the dynamic-import hook and `import.meta` for every host that installs hooks the documented way | forward every `HostHooks` method, delegating to `inner` (or store the host's hooks unwrapped and route rejections in the dispatchers) |

Confirmed by a probe crate at the pinned revision: with
`ctx.agent_mut().host_hooks = Some(Box::new(Specifiers))` the hook is called for
each resolution; with `ctx.set_host_hooks(Box::new(Specifiers))` it never is.

## The problem

`HostResolveImportedModule` in the engine looks a static specifier up in one flat
map, with no idea which module asked:

```rust
// crates/runtime/src/module.rs
pub fn host_resolve_imported_module(
    agent: &mut Agent,
    specifier: &JsString,
    attributes: &[(AttributeKey, JsString)],
) -> Result<Handle<SourceTextModule>, JsError> {
    let realm = agent.current_realm()?;
    if let Some(module) = realm.loaded_modules.borrow().get(specifier) {
        return Ok(*module);
    }
    let source = agent.host_modules.borrow().get(specifier).cloned() /* … */;
    // parse under `specifier`, cache under `specifier`
}
```

So `./util.js` means the same file in every module of the graph. A host that
hands the engine a directory tree therefore has to either flatten it, or define
its own rule that specifiers are root-relative and refuse the spellings a reader
would expect (Winter does the latter today — see "What the consumer does now").

Dynamic imports do not have this problem: `HostHooks::import_module_dynamically`
already receives the referrer's name. Only the static path has no hook at all.

## What the engine already has

- `Agent::host_modules: RefCell<HashMap<JsString, HostModuleSource>>` — the flat
  registry a host populates with `Agent::add_module` (documented as a
  "test/CLI hook").
- `Realm::loaded_modules: RefCell<HashMap<JsString, Handle<SourceTextModule>>>`,
  documented in `realm.rs` as "the Source Text Module Records keyed by
  **resolved** specifier". The cache's documented key is the resolved one; today
  the raw specifier is what goes in, because nothing resolves.
- `HostHooks` (`crates/runtime/src/host.rs`) with the dispatch helpers that route
  an `Agent`'s optional hooks to a `HostHooks` implementation, and
  `import_module_dynamically(specifier, referrer_name, phase, attributes)` as the
  precedent for a hook that takes a referrer.
- `parse_module(agent, specifier, name, source, attributes)`, where `name` is what
  a stack frame reports — a host-registered module is parsed with
  `Some(specifier)`.

Everything needed is already in place; the only missing piece is a mapping from
`(specifier, referrer)` to a registry key, and the will to use it.

## Proposal A (recommended): a specifier resolver on `HostHooks`

One new method. It maps a specifier and the importing module's name to the key the
registry and the cache should use. It cannot fail, and returning `None` means "use
the specifier as written" — which is today's behaviour, exactly.

```rust
// crates/runtime/src/host.rs
pub trait HostHooks: std::fmt::Debug {
    /// HostResolveImportedModule's name resolution (spec 16.6.1.1.2 step 1): the
    /// key `specifier`, written in `referrer`, names in the host's registry, or
    /// `None` for "resolve nothing, use the specifier as written".
    ///
    /// `referrer` is the name the engine has for the importing module — the
    /// `name` a host gave `Context::compile_module`, or the specifier a
    /// host-registered source was parsed under. It is `None` only for a module
    /// compiled without one.
    fn resolve_imported_specifier(
        &self,
        _specifier: &JsString,
        _referrer: Option<&JsString>,
    ) -> Option<JsString> {
        None
    }
}

/// The dispatch the engine's own resolution calls.
pub fn resolve_imported_specifier(
    agent: &Agent,
    specifier: &JsString,
    referrer: Option<&JsString>,
) -> Option<JsString> {
    match &agent.host_hooks {
        Some(hooks) => hooks.resolve_imported_specifier(specifier, referrer),
        None => None,
    }
}
```

`host_resolve_imported_module` then takes the referrer, resolves, and uses the
resolved key for the cache, the registry, the parse's specifier and the parse's
name:

```rust
pub fn host_resolve_imported_module(
    agent: &mut Agent,
    specifier: &JsString,
    referrer: Option<&JsString>,
    attributes: &[(AttributeKey, JsString)],
) -> Result<Handle<SourceTextModule>, JsError> {
    let key = resolve_imported_specifier(agent, specifier, referrer)
        .unwrap_or_else(|| specifier.clone());
    let realm = agent.current_realm()?;
    if let Some(module) = realm.loaded_modules.borrow().get(&key) {
        return Ok(*module);
    }
    let source = agent.host_modules.borrow().get(&key).cloned().ok_or_else(|| {
        JsError::new(
            ErrorKind::TypeError,
            format!("Cannot find module {}", key.to_string_lossy()),
        )
    })?;
    let text = String::from_utf8_lossy(&source.bytes).into_owned();
    let source_text = JsString::from_utf8(&text);
    let module = parse_module(agent, &key, Some(&key), &source_text, attributes)?;
    crux::heap::write_barrier_handle(&*realm, module);
    realm.loaded_modules.borrow_mut().insert(key, module);
    Ok(module)
}
```

### Call sites

Every caller gains one argument: the name of the module whose requests it is
walking. Each already holds that record.

| Function (`crates/runtime/src/module.rs`) | Referrer |
|---|---|
| `module_declaration_instantiation` (4 sites) | the `module` being linked |
| `module_evaluation` | the `module` being evaluated |
| `module_sync_ready`, `gather_async_transitive_dependencies` | the module in the async graph |
| `collect_exported_names`, `resolve_export` (3 sites) | the module whose entries are walked |
| `dynamic_import` (fallback when no hook answered) | the `referrer_name` it already has |

`module.name` is an `Option<JsString>` on the record, which is what the embed
surface already exposes as `JsModule::name()`. A host that wants resolution must
compile its entry with the name set — Winter does
(`compile_module(&entry.name, Some(&entry.name), &entry.text)`).

### Semantics

- **No hook, or `None`**: the specifier is the key, which is today's behaviour
  bit for bit.
- **Identity is the resolved key.** Two importers that resolve to the same key
  share one record, so a module is evaluated once; `realm.loaded_modules` holds
  one entry per resolved key.
- **A resolved key the host never registered** is `Cannot find module <key>` — the
  engine's existing sentence, carrying the key the author should have provided.
- **The name is the resolved key**, so a throw inside the module reports
  `lib/util.js` rather than `./util.js`.
- **Attributes and phase are untouched**: the hook maps names; what the request
  asks for (`with { type: "json" }`, source or defer phase) stays the request's.
- **Order**: resolution happens before the registry lookup and before the cache,
  so a raw key a host also registered is only reachable through the hook's answer.
  That is the documented rule; a host that wants raw keys back answers `None`.

### What it costs

One trait-object call and a string map lookup per resolution, and only when a host
installed hooks. The cache means a resolution happens once per key per realm.

### Tests to add upstream

The module tests already build small graphs (see the tests from `module.rs:3900`
onward); these mirror them:

1. `a_resolver_resolves_relative_to_the_referrer` — `lib/a.js` imports `./b.js`;
   the resolver maps `("./b.js", "lib/a.js")` to `lib/b.js`; evaluation reads
   `lib/b.js`'s export.
2. `two_importers_share_one_record` — `lib/a.js` and `lib/c.js` both import
   `./b.js` through the resolver; the module is evaluated once (a side effect
   counted), and `loaded_modules` has one `lib/b.js`.
3. `an_unresolved_key_is_a_missing_module` — the resolver answers
   `lib/missing.js`; the error names that key.
4. `without_a_hook_the_specifier_is_the_key` — the regression test for today.
5. `a_dynamic_import_uses_the_same_resolver` — `import("./b.js")` from `lib/a.js`,
   with the hook declining (`None` from `import_module_dynamically`) so the
   engine's fallback path resolves it; proves the referrer plumbing in both
   directions.
6. `the_name_is_the_resolved_key` — a throw inside `lib/b.js` reports
   `lib/b.js`.

## Proposal B: let the hook supply the bytes

If the goal is also to let a host synthesize a module (a compressed or encrypted
store, a module built at run time), the hook could return a source instead of a
key:

```rust
fn load_imported_module(
    &self,
    specifier: &JsString,
    referrer: Option<&JsString>,
    attributes: &[(AttributeKey, JsString)],
) -> Option<Result<HostModuleSource, JsError>>;
```

The engine parses and caches it exactly as it does a registered source. It is a
strict superset of A, and it makes the host responsible for module kind (JSON,
text, bytes) through `HostModuleSource`. It also costs a failed-resolution path
(`Some(Err(_))`) that A does not need. Proposed as a later step, if at all: A is
smaller, cannot fail, and covers every host whose modules are files.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Do nothing; a host rewrites specifiers before compiling | Puts a source transformation in the loader's trust path: offsets from `Module::requested_modules` are byte spans, string literals may carry escapes, and a bug corrupts code silently. It also cannot reach `export … from`, `import()` or a nested module it never compiled. |
| Register every module under every relative spelling it might be imported as | Combinatorial, and ambiguous as soon as two directories hold the same file name — the loader would have to pick one. |
| Make the hook return a module record | `HostHooks` methods take `&self` and cannot reach `&mut Agent`, which parsing needs; the hook would need a second registration API instead. |
| Resolve inside the host's own `import_module_dynamically` only | Dynamic imports already work; static ones are the ones that do not, and a mixed graph would resolve `import("./b.js")` and `import "./b.js"` differently. |

## What the consumer did, and what it does

Before the hook, Winter's loader registered an extension's modules flat, under
`./<root path>`, and compiled every file once before evaluating it to check what
it imports. Because nothing resolved relative to the importer, the loader had to

- tell authors that specifiers are root-relative (`./lib/util.js`, never
  `./util.js` inside `lib/`), and
- refuse a spelling whose sibling reading would be a different file, as
  "ambiguous", rather than resolving it one way.

With the hook, `crates/extensions` resolves `(specifier, referrer)` to a
root-relative key itself, registers modules under those plain keys (no `./`), and
has deleted both the rule and the ambiguity refusal: `./util.js` in `lib/a.js` is
`lib/util.js`, which is what a reader expects and what every other JavaScript
environment does. The one-file-per-key identity rule stays — two importers of the
same sibling share one record, so its side effects run once — and the "does not
ship" message still names the importing file, because the loader compiles each
file once to report an unregistered import at load rather than at first
evaluation. A bare specifier and one that climbs out of the root are refused at
load with the rule spelled out, and importing the entry is refused by name.

## Two smaller asks, from the same integration

1. **`initialize_import_meta_object` is unreachable from the embed surface.** The
   hook receives `crate::api::Module` and `crux::value::Value`, and neither
   `crux` nor a property setter on a raw `Value` is exported. A host therefore
   cannot fill `import.meta` at all. Handing the hook the embed types
   (`&JsModule`, `&JsObject`) — or re-exporting a typed setter — would make
   `import.meta.extension` possible.
2. **`Agent::add_module` is documented as a test/CLI hook** while being the only
   way to register a module, reached through `Context::agent_mut()`. A
   `Context::add_module_source(specifier, bytes)` (with the module kind, for JSON
   and text) would make that a supported embedding API rather than a dependency on
   an internal hook.
3. **`JsModule` has no `stalled_top_level_await_modules`**, which `api::Module`
   does. A host judges a module's evaluation by its promise today; exposing the
   same question on `JsModule` would let it ask directly.

## Open questions for the engine author

- Name: `resolve_imported_specifier` (proposal) against
  `host_resolve_specifier` or `resolve_module_name`.
- Should the hook also see the import attributes and phase, for a host that maps
  `./data` differently by `with { type: "json" }`? Proposal A says no, so that a
  resolver stays a pure name mapping; adding the parameters later is additive for
  the trait's default method.
- Should `Context::register_module` (the embed surface's eager registration) also
  be consulted after resolution? Today it writes `realm.loaded_modules` directly;
  with the hook, a host could register under resolved keys and never need it.
- Whether to keep `Agent::add_module`'s name or fold it into the `Context` API
  once (2) exists.
