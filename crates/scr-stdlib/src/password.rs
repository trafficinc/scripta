//! Bounded Argon2id v19 password hashing. No public random/token primitives.
use argon2::{
    Algorithm, Argon2, Block, Params, Version,
    password_hash::{Output, ParamsString, PasswordHash, SaltString},
};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use zeroize::Zeroizing;

const SALT_BYTES: usize = 16;
const OUTPUT_BYTES: usize = 32;
const MAX_MEMORY_KIB: u32 = 262_144;
const MAX_ITERATIONS: u32 = 32;
const MAX_PARALLELISM: u32 = 16;

/// Trusted host policy. Scripts cannot raise costs or supplied-hash ceilings.
/// Memory is per call; concurrent web workers multiply that allocation.
#[derive(Debug, Clone)]
pub struct PasswordConfig {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub max_memory_kib: u32,
    pub max_iterations: u32,
    pub max_parallelism: u32,
    /// Ceiling on m*t, measured in KiB-passes; parallelism does not multiply m.
    pub max_work_kib: u64,
    pub max_password_bytes: usize,
    pub max_hash_bytes: usize,
}
impl Default for PasswordConfig {
    fn default() -> Self {
        Self {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
            max_memory_kib: 65_536,
            max_iterations: 10,
            max_parallelism: 4,
            max_work_kib: 262_144,
            max_password_bytes: 1024,
            max_hash_bytes: 512,
        }
    }
}
fn error(message: &str) -> NativeError {
    NativeError::new("PasswordError", message)
}
impl PasswordConfig {
    fn costs(&self, m: u32, t: u32, p: u32) -> NativeResult<()> {
        if m > self.max_memory_kib
            || t > self.max_iterations
            || p > self.max_parallelism
            || u64::from(m) * u64::from(t) > self.max_work_kib
        {
            return Err(error("Argon2 costs exceed host policy"));
        }
        Ok(())
    }
    fn prepare(&self) -> NativeResult<Params> {
        if self.max_memory_kib < 8
            || self.max_memory_kib > MAX_MEMORY_KIB
            || !(1..=MAX_ITERATIONS).contains(&self.max_iterations)
            || !(1..=MAX_PARALLELISM).contains(&self.max_parallelism)
            || self.max_work_kib == 0
            || self.max_work_kib > u64::from(MAX_MEMORY_KIB) * u64::from(MAX_ITERATIONS)
            || !(1..=1024 * 1024).contains(&self.max_password_bytes)
            || !(1..=4096).contains(&self.max_hash_bytes)
        {
            return Err(error("invalid Argon2 host policy"));
        }
        self.costs(self.memory_kib, self.iterations, self.parallelism)?;
        let params = Params::new(
            self.memory_kib,
            self.iterations,
            self.parallelism,
            Some(OUTPUT_BYTES),
        )
        .map_err(|_| error("invalid Argon2 hashing parameters"))?;
        if render(
            &params,
            &SaltString::encode_b64(&[0; SALT_BYTES])
                .map_err(|_| error("Argon2 salt encoding failed"))?,
            &[0; OUTPUT_BYTES],
        )?
        .len()
            > self.max_hash_bytes
        {
            return Err(error(
                "encoded hash byte ceiling cannot hold the configured output",
            ));
        }
        Ok(params)
    }
}
fn render(params: &Params, salt: &SaltString, output: &[u8]) -> NativeResult<String> {
    Ok(PasswordHash {
        algorithm: Algorithm::Argon2id.ident(),
        version: Some(19),
        params: ParamsString::try_from(params)
            .map_err(|_| error("Argon2 parameter encoding failed"))?,
        salt: Some(salt.as_salt()),
        hash: Some(Output::new(output).map_err(|_| error("Argon2 output encoding failed"))?),
    }
    .to_string())
}
struct Parsed<'a> {
    hash: PasswordHash<'a>,
    params: Params,
    salt: [u8; 64],
    salt_len: usize,
    output_len: usize,
}
fn parse<'a>(encoded: &'a str, config: &PasswordConfig) -> NativeResult<Option<Parsed<'a>>> {
    if encoded.len() > config.max_hash_bytes {
        return Err(error("encoded hash exceeds host byte limit"));
    }
    let Ok(hash) = PasswordHash::new(encoded) else {
        return Ok(None);
    };
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(19)
        || hash.params.iter().count() != 3
        || hash
            .params
            .iter()
            .any(|(name, _)| !matches!(name.as_str(), "m" | "t" | "p"))
    {
        return Ok(None);
    }
    let (Some(m), Some(t), Some(p)) = (
        hash.params.get_decimal("m"),
        hash.params.get_decimal("t"),
        hash.params.get_decimal("p"),
    ) else {
        return Ok(None);
    };
    let Some(output) = &hash.hash else {
        return Ok(None);
    };
    if !(16..=64).contains(&output.len()) {
        return Ok(None);
    }
    config.costs(m, t, p)?;
    let Ok(params) = Params::new(m, t, p, Some(output.len())) else {
        return Ok(None);
    };
    let Some(salt) = hash.salt else {
        return Ok(None);
    };
    let mut salt_bytes = [0; 64];
    let Ok(decoded) = salt.decode_b64(&mut salt_bytes) else {
        return Ok(None);
    };
    let salt_len = decoded.len();
    if salt_len < 8 {
        return Ok(None);
    }
    let output_len = output.len();
    Ok(Some(Parsed {
        hash,
        params,
        salt: salt_bytes,
        salt_len,
        output_len,
    }))
}
fn compute(
    context: &RuntimeContext<'_>,
    password: &str,
    salt: &[u8],
    params: &Params,
    output_len: usize,
) -> NativeResult<Zeroizing<[u8; 64]>> {
    context.check_cancelled()?;
    let count = params.block_count();
    let mut memory = Zeroizing::new(Vec::<Block>::new());
    memory
        .try_reserve_exact(count)
        .map_err(|_| crate::bytes::resource("Argon2 working memory allocation failed"))?;
    for index in 0..count {
        if index % 1024 == 0 {
            context.check_cancelled()?;
        }
        memory.push(Block::new());
    }
    let mut output = Zeroizing::new([0; 64]);
    context.check_cancelled()?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone())
        .hash_password_into_with_memory(
            password.as_bytes(),
            salt,
            &mut output[..output_len],
            &mut *memory,
        )
        .map_err(|_| error("Argon2 hashing failed"))?;
    // The backend cannot be cooperatively interrupted during its computation.
    // Web hosts can terminate the isolated worker at the request deadline.
    context.check_cancelled()?;
    Ok(output)
}
fn invoke(
    name: &str,
    context: &RuntimeContext<'_>,
    args: &[Value],
    config: &PasswordConfig,
    params: &Params,
    encoded_len: usize,
) -> NativeResult {
    context.check_cancelled()?;
    let text = |index: usize| match &args[index] {
        Value::String(s) => s.as_str(),
        _ => unreachable!("validated string input"),
    };
    if (name == "hash" || name == "verify") && text(0).len() > config.max_password_bytes {
        return Err(error("input exceeds host byte limit"));
    }
    crate::bytes::payload(context, if name == "hash" { encoded_len } else { 0 })?;
    match name {
        "hash" => {
            let mut salt = [0; SALT_BYTES];
            getrandom::fill(&mut salt).map_err(|_| error("Argon2 salt generation failed"))?;
            let salt_string =
                SaltString::encode_b64(&salt).map_err(|_| error("Argon2 salt encoding failed"))?;
            let output = compute(context, text(0), &salt, params, OUTPUT_BYTES)?;
            let result = render(params, &salt_string, &output[..OUTPUT_BYTES])?;
            context.check_cancelled()?;
            Ok(Value::String(result))
        }
        "verify" => {
            let Some(parsed) = parse(text(1), config)? else {
                return Ok(Value::Bool(false));
            };
            let computed = compute(
                context,
                text(0),
                &parsed.salt[..parsed.salt_len],
                &parsed.params,
                parsed.output_len,
            )?;
            let actual = Output::new(&computed[..parsed.output_len])
                .map_err(|_| error("Argon2 verification failed"))?;
            // RustCrypto password-hash Output equality is constant-time, matching
            // its PasswordVerifier implementation. Do not compare encoded text.
            let matches = parsed
                .hash
                .hash
                .as_ref()
                .is_some_and(|expected| expected == &actual);
            context.check_cancelled()?;
            Ok(Value::Bool(matches))
        }
        "needsRehash" => {
            let Some(parsed) = parse(text(0), config)? else {
                return Ok(Value::Bool(true));
            };
            let differs = parsed.params.m_cost() != config.memory_kib
                || parsed.params.t_cost() != config.iterations
                || parsed.params.p_cost() != config.parallelism
                || parsed.output_len != OUTPUT_BYTES
                || parsed.salt_len != SALT_BYTES;
            context.check_cancelled()?;
            Ok(Value::Bool(differs))
        }
        _ => unreachable!(),
    }
}
pub(crate) fn register(runtime: &mut Runtime, config: PasswordConfig) -> NativeResult<()> {
    let params = config.prepare()?;
    let encoded_len = render(
        &params,
        &SaltString::encode_b64(&[0; SALT_BYTES])
            .map_err(|_| error("Argon2 salt encoding failed"))?,
        &[0; OUTPUT_BYTES],
    )?
    .len();
    for name in ["hash", "verify", "needsRehash"] {
        let config = config.clone();
        let params = params.clone();
        let signature = NativeSignature::new(
            vec![NativeType::String; if name == "verify" { 2 } else { 1 }],
            if name == "hash" {
                NativeType::String
            } else {
                NativeType::Bool
            },
        );
        runtime.register(
            format!("Password::{name}"),
            NativeFunction::new(signature, move |context, args| {
                invoke(name, context, args, &config, &params, encoded_len)
            }),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        mem::size_of,
        sync::{Arc, atomic::AtomicBool},
    };

    #[test]
    fn direct_native_callbacks_check_cancellation_and_scalar_and_hash_result_budgets() {
        let config = PasswordConfig {
            memory_kib: 32,
            iterations: 2,
            parallelism: 1,
            ..Default::default()
        };
        let params = config.prepare().unwrap();
        let salt = SaltString::encode_b64(&[0; SALT_BYTES]).unwrap();
        let encoded_len = render(&params, &salt, &[0; OUTPUT_BYTES]).unwrap().len();
        let mut settings = ContextOptions::default();
        settings.permissions.allowed.clear();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let mut context = RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut errors,
            runtime_identity: None,
            cancellation: Some(Arc::new(AtomicBool::new(true))),
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        };
        for (name, args) in [
            ("hash", vec![Value::String("public".into())]),
            (
                "verify",
                vec![
                    Value::String("public".into()),
                    Value::String("not a hash".into()),
                ],
            ),
            ("needsRehash", vec![Value::String("not a hash".into())]),
        ] {
            assert_eq!(
                invoke(name, &context, &args, &config, &params, encoded_len)
                    .unwrap_err()
                    .kind,
                "Interrupted"
            );
            context.cancellation = None;
            context.max_value_bytes = Some(size_of::<Value>() - 1);
            assert_eq!(
                invoke(name, &context, &args, &config, &params, encoded_len)
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            context.max_value_bytes =
                Some(size_of::<Value>() + if name == "hash" { encoded_len } else { 0 });
            assert!(invoke(name, &context, &args, &config, &params, encoded_len).is_ok());
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            context.max_value_bytes = None;
        }
    }
}
