//! SHA-256 and HMAC-SHA256 through ring's bounded incremental backends.
use crate::bytes::{payload, resource};
use ring::{digest, hmac};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::RefCell,
    mem::size_of,
    rc::{Rc, Weak},
};
use zeroize::Zeroizing;

const CHUNK_BYTES: usize = 16 * 1024;
const DIGEST_BYTES: usize = 32;
const MAX_HANDLES: usize = 64;
// SHA-256 encodes its input bit length in 64 bits. HMAC's inner hash also
// consumes one 64-byte key-pad block before the message.
const MAX_SHA_BYTES: u64 = u64::MAX / 8;

fn closed() -> NativeError {
    NativeError::new("HashError", "HashContext is closed")
}
fn limit(message: &str) -> NativeError {
    NativeError::new("HashLimitError", message)
}

enum Backend {
    Sha256(digest::Context),
    HmacSha256(hmac::Context),
}
struct State {
    backend: Backend,
    bytes: u64,
}
impl State {
    fn sha256() -> Self {
        Self {
            backend: Backend::Sha256(digest::Context::new(&digest::SHA256)),
            bytes: 0,
        }
    }
    fn hmac(context: &RuntimeContext<'_>, key: &[u8]) -> NativeResult<Self> {
        context.check_cancelled()?;
        // RFC 2104 normalizes keys longer than the block size with SHA-256.
        // Do this incrementally so a large script key remains interruptible.
        let normalized;
        let key = if key.len() > 64 {
            let mut state = Self::sha256();
            state.update(key, || context.check_cancelled())?;
            normalized = Zeroizing::new(state.finish());
            normalized.as_slice()
        } else {
            key
        };
        let signing_key = hmac::Key::new(hmac::HMAC_SHA256, key);
        Ok(Self {
            backend: Backend::HmacSha256(hmac::Context::with_key(&signing_key)),
            bytes: 0,
        })
    }
    fn update(
        &mut self,
        data: &[u8],
        mut checkpoint: impl FnMut() -> NativeResult<()>,
    ) -> NativeResult<()> {
        let maximum = match self.backend {
            Backend::Sha256(_) => MAX_SHA_BYTES,
            Backend::HmacSha256(_) => MAX_SHA_BYTES - 64,
        };
        let next = u64::try_from(data.len())
            .ok()
            .and_then(|n| self.bytes.checked_add(n))
            .filter(|n| *n <= maximum)
            .ok_or_else(|| limit("SHA-256 input length limit exceeded"))?;
        for chunk in data.chunks(CHUNK_BYTES) {
            checkpoint()?;
            match &mut self.backend {
                Backend::Sha256(state) => state.update(chunk),
                Backend::HmacSha256(state) => state.update(chunk),
            }
        }
        checkpoint()?;
        self.bytes = next;
        Ok(())
    }
    fn finish(self) -> [u8; DIGEST_BYTES] {
        match self.backend {
            Backend::Sha256(state) => state
                .finish()
                .as_ref()
                .try_into()
                .expect("SHA-256 digest size"),
            Backend::HmacSha256(state) => state
                .sign()
                .as_ref()
                .try_into()
                .expect("HMAC-SHA256 tag size"),
        }
    }
}

// A fixed backend-state slot remains charged even when closed, so a handle's
// logical footprint cannot grow during update or change through aliases.
const HANDLE_BYTES: usize = size_of::<State>();
struct Handle {
    state: RefCell<Option<State>>,
    owner: Rc<()>,
    algorithm: &'static str,
}
impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashContext")
            .field("algorithm", &self.algorithm)
            .field("closed", &self.state.borrow().is_none())
            .finish()
    }
}
impl NativeObject for Handle {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "HashContext"
    }
    fn payload_bytes(&self) -> usize {
        HANDLE_BYTES
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.state.borrow().is_none())),
            "algorithm" => Ok(Value::String(self.algorithm.into())),
            _ => Err(format!("unknown HashContext property {name}")),
        }
    }
}
impl Handle {
    fn update(
        &self,
        data: &[u8],
        checkpoint: impl FnMut() -> NativeResult<()>,
    ) -> NativeResult<()> {
        let mut state = self.state.borrow_mut();
        let result = state.as_mut().ok_or_else(closed)?.update(data, checkpoint);
        // Never leave a partly updated context usable after interruption.
        if result.as_ref().is_err_and(|e| e.kind == "Interrupted") {
            state.take();
        }
        result
    }
}

struct Registration {
    owner: Rc<()>,
    runtime: Weak<()>,
    handles: RefCell<Vec<Weak<Handle>>>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        // Host-retained Values must not retain active HMAC state past teardown.
        for handle in self
            .handles
            .get_mut()
            .drain(..)
            .filter_map(|weak| weak.upgrade())
        {
            handle.state.borrow_mut().take();
        }
    }
}
impl Registration {
    fn check_runtime(&self, context: &RuntimeContext<'_>) -> NativeResult<()> {
        if !context
            .runtime_identity
            .as_ref()
            .is_some_and(|identity| self.runtime.ptr_eq(&Rc::downgrade(identity)))
        {
            return Err(NativeError::type_error(
                "hash registration belongs to another runtime",
            ));
        }
        Ok(())
    }
    fn handle<'a>(&self, value: &'a Value) -> NativeResult<&'a Handle> {
        let Value::NativeObject(object) = value else {
            return Err(NativeError::type_error("expected HashContext"));
        };
        let handle = object
            .as_any()
            .and_then(|o| o.downcast_ref::<Handle>())
            .ok_or_else(|| NativeError::type_error("expected real HashContext"))?;
        if !Rc::ptr_eq(&handle.owner, &self.owner) {
            return Err(NativeError::type_error(
                "HashContext belongs to another registration",
            ));
        }
        Ok(handle)
    }
    fn init(&self, context: &RuntimeContext<'_>, key: Option<&[u8]>) -> NativeResult<Value> {
        context.check_cancelled()?;
        payload(context, HANDLE_BYTES)?;
        let mut handles = self.handles.borrow_mut();
        handles.retain(|weak| weak.upgrade().is_some_and(|h| h.state.borrow().is_some()));
        if handles.len() >= MAX_HANDLES {
            return Err(limit("open HashContext limit exceeded"));
        }
        handles
            .try_reserve(1)
            .map_err(|_| resource("hash handle allocation failed"))?;
        let state = match key {
            Some(key) => State::hmac(context, key)?,
            None => State::sha256(),
        };
        let handle = Rc::new(Handle {
            state: RefCell::new(Some(state)),
            owner: self.owner.clone(),
            algorithm: if key.is_some() {
                "hmac-sha256"
            } else {
                "sha256"
            },
        });
        handles.push(Rc::downgrade(&handle));
        Ok(Value::NativeObject(handle))
    }
}
fn data(value: &Value) -> &[u8] {
    let Value::Bytes(data) = value else {
        unreachable!()
    };
    data
}
fn buffer(context: &RuntimeContext<'_>) -> NativeResult<Vec<u8>> {
    payload(context, DIGEST_BYTES)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(DIGEST_BYTES)
        .map_err(|_| resource("digest allocation failed"))?;
    Ok(result)
}
fn invoke(
    registration: &Registration,
    name: &str,
    context: &RuntimeContext<'_>,
    args: &[Value],
) -> NativeResult<Value> {
    context.check_cancelled()?;
    // Resource APIs check the calling runtime as well as the handle's owner.
    if !matches!(name, "sha256" | "hmacSha256") {
        registration.check_runtime(context)?;
    }
    match name {
        "sha256" | "hmacSha256" => {
            let mut result = buffer(context)?;
            let mut state = if name == "sha256" {
                State::sha256()
            } else {
                State::hmac(context, data(&args[0]))?
            };
            state.update(data(&args[usize::from(name == "hmacSha256")]), || {
                context.check_cancelled()
            })?;
            result.extend_from_slice(&state.finish());
            Ok(Value::Bytes(result.into()))
        }
        "sha256Init" => registration.init(context, None),
        "hmacSha256Init" => registration.init(context, Some(data(&args[0]))),
        "update" => {
            let handle = registration.handle(&args[0])?;
            payload(context, 0)?;
            handle.update(data(&args[1]), || context.check_cancelled())?;
            Ok(Value::Null)
        }
        "finalize" => {
            let handle = registration.handle(&args[0])?;
            let mut state = handle.state.borrow_mut();
            if state.is_none() {
                return Err(closed());
            }
            // Preflight and reserve before consuming the handle so resource
            // failure preserves state for a later retry or explicit close.
            let mut result = buffer(context)?;
            result.extend_from_slice(&state.take().expect("open hash state").finish());
            Ok(Value::Bytes(result.into()))
        }
        "close" => {
            let handle = registration.handle(&args[0])?;
            payload(context, 0)?;
            handle.state.borrow_mut().take();
            Ok(Value::Null)
        }
        _ => unreachable!(),
    }
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bytes, Null, Object};
    let registration = Rc::new(Registration {
        owner: Rc::new(()),
        runtime: Rc::downgrade(&runtime.native_identity()),
        handles: RefCell::new(Vec::new()),
    });
    for (name, parameters, returns) in [
        ("sha256", vec![Bytes], Bytes),
        ("hmacSha256", vec![Bytes, Bytes], Bytes),
        ("sha256Init", vec![], Object("HashContext")),
        ("hmacSha256Init", vec![Bytes], Object("HashContext")),
        ("update", vec![Object("HashContext"), Bytes], Null),
        ("finalize", vec![Object("HashContext")], Bytes),
        ("close", vec![Object("HashContext")], Null),
    ] {
        let registration = registration.clone();
        runtime.register(
            format!("Hash::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| invoke(&registration, name, context, args),
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: Some(Rc::new(())),
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }
    fn registration(context: &RuntimeContext<'_>) -> Registration {
        Registration {
            owner: Rc::new(()),
            runtime: Rc::downgrade(context.runtime_identity.as_ref().unwrap()),
            handles: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn exact_budgets_reject_growth_before_creating_or_consuming_state() {
        with_context(|context| {
            for key in [None, Some(b"key".as_slice())] {
                let registration = registration(context);
                context.max_value_bytes = Some(size_of::<Value>() + HANDLE_BYTES - 1);
                assert_eq!(
                    registration.init(context, key).unwrap_err().kind,
                    "ResourceLimitError"
                );
                assert!(registration.handles.borrow().is_empty());
                context.max_value_bytes = Some(size_of::<Value>() + HANDLE_BYTES);
                let value = registration.init(context, key).unwrap();
                let handle = registration.handle(&value).unwrap();
                assert_eq!(handle.payload_bytes(), HANDLE_BYTES);
                assert_eq!(value.type_name(), "HashContext");
                // Output preflight must fail before finalize consumes state.
                context.max_value_bytes = Some(size_of::<Value>() + DIGEST_BYTES - 1);
                assert_eq!(
                    invoke(
                        &registration,
                        "finalize",
                        context,
                        std::slice::from_ref(&value)
                    )
                    .unwrap_err()
                    .kind,
                    "ResourceLimitError"
                );
                assert!(handle.state.borrow().is_some());
                context.max_value_bytes = Some(size_of::<Value>() + DIGEST_BYTES);
                let result = invoke(
                    &registration,
                    "finalize",
                    context,
                    std::slice::from_ref(&value),
                )
                .unwrap();
                let Value::Bytes(result) = result else {
                    panic!()
                };
                assert_eq!(result.len(), DIGEST_BYTES);
                assert!(handle.state.borrow().is_none());
                assert_eq!(handle.payload_bytes(), HANDLE_BYTES);
            }
        });
    }

    #[test]
    fn every_callback_checks_cancellation_before_mutation() {
        with_context(|context| {
            let registration = registration(context);
            let handle = registration.init(context, Some(b"key")).unwrap();
            let data = Value::Bytes(b"data".as_slice().into());
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            for (name, args) in [
                ("sha256", vec![data.clone()]),
                ("hmacSha256", vec![data.clone(), data.clone()]),
                ("sha256Init", vec![]),
                ("hmacSha256Init", vec![data.clone()]),
                ("update", vec![handle.clone(), data]),
                ("finalize", vec![handle.clone()]),
                ("close", vec![handle.clone()]),
            ] {
                assert_eq!(
                    invoke(&registration, name, context, &args)
                        .unwrap_err()
                        .kind,
                    "Interrupted",
                    "{name}"
                );
                assert_eq!(
                    registration
                        .handle(&handle)
                        .unwrap()
                        .state
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .bytes,
                    0
                );
                assert_eq!(registration.handles.borrow().len(), 1);
            }
            // Key preprocessing must also check cancellation directly.
            assert_eq!(
                State::hmac(context, &vec![7; CHUNK_BYTES + 1])
                    .err()
                    .unwrap()
                    .kind,
                "Interrupted"
            );
        });
    }

    #[test]
    fn cancellation_between_chunks_closes_partly_updated_contexts() {
        with_context(|context| {
            let registration = registration(context);
            for key in [None, Some(b"key".as_slice())] {
                for cancel_at in 0..4 {
                    let value = registration.init(context, key).unwrap();
                    let handle = registration.handle(&value).unwrap();
                    let mut calls = 0;
                    let result = handle.update(&vec![0xff; CHUNK_BYTES * 2 + 7], || {
                        let current = calls;
                        calls += 1;
                        if current == cancel_at {
                            Err(NativeError::new("Interrupted", "execution interrupted"))
                        } else {
                            Ok(())
                        }
                    });
                    assert_eq!(result.unwrap_err().kind, "Interrupted");
                    assert_eq!(calls, cancel_at + 1);
                    assert!(handle.state.borrow().is_none());
                }
            }
        });
    }

    #[test]
    fn input_length_ceiling_is_checked_before_backend_mutation() {
        with_context(|context| {
            for (mut state, maximum) in [
                (State::sha256(), MAX_SHA_BYTES),
                (State::hmac(context, b"key").unwrap(), MAX_SHA_BYTES - 64),
            ] {
                // Synthetic counters exercise limits impossible to allocate in
                // a unit test; do not finalize these synthetic-length states.
                state.bytes = maximum - 1;
                state.update(b"x", || Ok(())).unwrap();
                assert_eq!(state.bytes, maximum);
                state.update(b"", || Ok(())).unwrap();
                assert_eq!(
                    state
                        .update(b"x", || panic!("limit must precede update"))
                        .unwrap_err()
                        .kind,
                    "HashLimitError"
                );
                assert_eq!(state.bytes, maximum);
                state.bytes = u64::MAX;
                assert_eq!(
                    state.update(b"x", || Ok(())).unwrap_err().kind,
                    "HashLimitError"
                );
            }
        });
    }

    #[test]
    fn handle_apis_reject_a_foreign_calling_runtime_before_mutation() {
        with_context(|context| {
            let registration = registration(context);
            let handle = registration.init(context, Some(b"key")).unwrap();
            let original_identity = context.runtime_identity.take();
            for identity in [None, Some(Rc::new(()))] {
                context.runtime_identity = identity;
                let data = Value::Bytes(b"x".as_slice().into());
                for (name, args) in [
                    ("sha256Init", vec![]),
                    ("hmacSha256Init", vec![data.clone()]),
                    ("update", vec![handle.clone(), data]),
                    ("finalize", vec![handle.clone()]),
                    ("close", vec![handle.clone()]),
                ] {
                    let error = invoke(&registration, name, context, &args).unwrap_err();
                    assert_eq!(error.kind, "TypeError", "{name}");
                    assert!(error.message.contains("another runtime"));
                    assert_eq!(
                        registration
                            .handle(&handle)
                            .unwrap()
                            .state
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .bytes,
                        0
                    );
                }
            }
            context.runtime_identity = original_identity;
            invoke(&registration, "finalize", context, &[handle]).unwrap();
        });
    }
}
