;; A wasm-only Winter extension, in the ABI `docs/extensions-scope.md` records.
;;
;; It imports only what it uses, all of it from the `winter` namespace, and
;; registers during `winter_init` — a start function would run before the host
;; owns the module's memory. `extension.wasm` beside this file is this module
;; compiled; the ignored `regenerate_the_wasm_fixture` test writes it.
(module
  (import "winter" "log" (func $log (param i32 i32)))
  (import "winter" "notify" (func $notify (param i32 i32 i32 i32)))
  (import "winter" "register_command"
    (func $register_command (param i32 i32 i32 i32) (result i32)))
  (import "winter" "register_event" (func $register_event (param i32 i32) (result i32)))
  (import "winter" "snapshot" (func $snapshot (param i32 i32) (result i32)))

  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))

  ;; A bump allocator for the strings the host writes in. The fixture never
  ;; frees, which is what a fixture is for.
  (func $alloc (export "winter_alloc") (param $len i32) (result i32)
    (local $at i32)
    (local.set $at (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $at))

  (func (export "winter_abi") (result i32)
    (i32.const 1))

  (func (export "winter_init") (result i32)
    (local $buffer i32)
    (call $log (i32.const 0) (i32.const 15))
    (call $log (i32.const 16) (i32.const 9))
    (drop (call $register_command
      (i32.const 32) (i32.const 10)
      (i32.const 48) (i32.const 15)))
    (drop (call $register_event (i32.const 64) (i32.const 15)))
    ;; There is no shell to read during `winter_init`, so the snapshot is the
    ;; four bytes `null`; logging them is what proves the rule.
    (local.set $buffer (call $alloc (i32.const 128)))
    (drop (call $snapshot (local.get $buffer) (i32.const 128)))
    (call $log (local.get $buffer) (i32.const 4))
    (i32.const 0))

  (func (export "winter_command") (param $id i32) (param $len i32) (result i32)
    (call $log (i32.const 80) (i32.const 18))
    (i32.const 0))

  ;; Logs the payload the host wrote in, then the head of the snapshot it can
  ;; read back — the module-to-host and host-to-module directions of the string
  ;; crossing, and the read surface in one call.
  (func (export "winter_event")
    (param $name i32) (param $name_len i32) (param $payload i32) (param $payload_len i32)
    (result i32)
    (local $buffer i32)
    (call $log (local.get $payload) (local.get $payload_len))
    (local.set $buffer (call $alloc (i32.const 128)))
    (drop (call $snapshot (local.get $buffer) (i32.const 128)))
    (call $log (local.get $buffer) (i32.const 64))
    (call $notify
      (i32.const 104) (i32.const 12)
      (i32.const 120) (i32.const 16))
    (i32.const 0))

  (data (i32.const 0) "hello from wasm")
  (data (i32.const 16) "caf\c3\a9 \e2\9c\93")
  (data (i32.const 32) "hello-wasm")
  (data (i32.const 48) "Hello from wasm")
  (data (i32.const 64) "commandFinished")
  (data (i32.const 80) "A wasm command ran")
  (data (i32.const 104) "Wasm fixture")
  (data (i32.const 120) "a command failed"))
