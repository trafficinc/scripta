//! Output accounting and cancellation. Buffering and transport remain host-owned.
use crate::{NativeError, NativeResult, RuntimeOptions};
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Native output exposes byte accounting without downcasting a host writer.
pub trait ScriptOutput: Write {
    fn generated_bytes(&self) -> Option<usize> {
        None
    }
    fn remaining_bytes(&self) -> Option<usize> {
        None
    }
}
impl ScriptOutput for Vec<u8> {}
pub(crate) struct ExecutionOutput<'a, W> {
    target: &'a mut W,
    generated: usize,
    limit: Option<usize>,
    cancellation: Option<Arc<AtomicBool>>,
    failed: bool,
}
impl<'a, W: Write> ExecutionOutput<'a, W> {
    pub fn new(target: &'a mut W, options: &RuntimeOptions) -> Self {
        Self {
            target,
            generated: 0,
            limit: options.max_output_bytes,
            cancellation: options.interrupt_flag.clone(),
            failed: false,
        }
    }
    fn check(&self) -> NativeResult<()> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
        {
            return Err(NativeError::new("Interrupted", "execution interrupted"));
        }
        if self.failed {
            return Err(NativeError::new("OutputError", "output has failed"));
        }
        Ok(())
    }
    pub fn finish(&self) -> NativeResult<()> {
        self.check()
    }
}
impl<W: Write> Write for ExecutionOutput<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check().map_err(io::Error::other)?;
        let total = self
            .generated
            .checked_add(bytes.len())
            .filter(|n| self.limit.is_none_or(|limit| *n <= limit));
        let Some(total) = total else {
            self.failed = true;
            return Err(io::Error::other("cumulative output byte limit exceeded"));
        };
        if let Err(error) = self.target.write_all(bytes) {
            self.failed = true;
            return Err(error);
        }
        self.generated = total;
        Ok(bytes.len())
    }
    // Flushing is a host/transport operation, separate from script evaluation.
    fn flush(&mut self) -> io::Result<()> {
        self.check().map_err(io::Error::other)?;
        if let Err(error) = self.target.flush() {
            self.failed = true;
            return Err(error);
        }
        Ok(())
    }
}
impl<W: Write> ScriptOutput for ExecutionOutput<'_, W> {
    fn generated_bytes(&self) -> Option<usize> {
        Some(self.generated)
    }
    fn remaining_bytes(&self) -> Option<usize> {
        self.limit.map(|n| n.saturating_sub(self.generated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounting_includes_native_writes_and_cancellation_stops_output_and_flush() {
        let flag = Arc::new(AtomicBool::new(false));
        let options = RuntimeOptions {
            max_output_bytes: Some(4),
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        };
        let mut target = vec![];
        let mut output = ExecutionOutput::new(&mut target, &options);
        assert_eq!(output.generated_bytes(), Some(0));
        assert_eq!(output.remaining_bytes(), Some(4));
        output.write_all(b"1234").unwrap();
        assert_eq!(output.generated_bytes(), Some(4));
        assert_eq!(output.remaining_bytes(), Some(0));
        flag.store(true, Ordering::Release);
        assert!(output.write_all(b"x").is_err());
        assert!(output.flush().is_err());
        assert_eq!(output.finish().unwrap_err().kind, "Interrupted");
        drop(output);
        assert_eq!(target, b"1234");
    }
}
