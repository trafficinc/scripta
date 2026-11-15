//! Generic application output buffers. Transport implementations remain host-owned.
use crate::{NativeError, NativeResult, RuntimeOptions, Value};
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy)]
pub enum OutputAction {
    Start,
    Flush,
    EndFlush,
    Clean,
    EndClean,
    Contents,
    GetClean,
    GetFlush,
    Length,
    Level,
}
/// Native output exposes buffer control without downcasting a host's writer.
pub trait ScriptOutput: Write {
    fn generated_bytes(&self) -> Option<usize> {
        None
    }
    fn remaining_bytes(&self) -> Option<usize> {
        None
    }
    fn control(&mut self, _action: OutputAction) -> NativeResult<Value> {
        Err(NativeError::new(
            "OutputError",
            "output buffers require a runtime execution context",
        ))
    }
}
impl ScriptOutput for Vec<u8> {}
pub(crate) struct BufferedOutput<'a, W> {
    target: &'a mut W,
    buffers: Vec<Vec<u8>>,
    pending: usize,
    generated: usize,
    limit: Option<usize>,
    buffer_limit: usize,
    depth: usize,
    value_limit: Option<usize>,
    cancellation: Option<Arc<AtomicBool>>,
    failed: bool,
}
impl<'a, W: Write> BufferedOutput<'a, W> {
    pub fn new(target: &'a mut W, options: &RuntimeOptions) -> NativeResult<Self> {
        if options.max_output_buffers > 128 || options.max_output_buffer_bytes > 64 * 1024 * 1024 {
            return Err(NativeError::new(
                "OutputError",
                "invalid output buffer limits",
            ));
        }
        Ok(Self {
            target,
            buffers: vec![],
            pending: 0,
            generated: 0,
            limit: options.max_output_bytes,
            buffer_limit: options.max_output_buffer_bytes,
            depth: options.max_output_buffers,
            value_limit: options.max_value_bytes,
            cancellation: options.interrupt_flag.clone(),
            failed: false,
        })
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
    fn value(&self, bytes: usize) -> NativeResult<()> {
        if self
            .value_limit
            .is_some_and(|limit| bytes.saturating_add(size_of::<Value>()) > limit)
        {
            return Err(NativeError::new(
                "ResourceLimitError",
                "output result exceeds value byte limit",
            ));
        }
        Ok(())
    }
    fn drain(&mut self, end: bool) -> NativeResult<()> {
        if self.buffers.is_empty() {
            return Err(NativeError::new("OutputError", "no active output buffer"));
        }
        let bytes = self.buffers.pop().unwrap();
        self.pending -= bytes.len();
        let result = self.append(&bytes);
        if !end {
            self.buffers.push(vec![])
        }
        result.map_err(Into::into)
    }
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Some(buffer) = self.buffers.last_mut() {
            if bytes.len() > self.buffer_limit.saturating_sub(self.pending) {
                self.failed = true;
                return Err(io::Error::other("output buffer byte limit exceeded"));
            }
            if buffer.try_reserve(bytes.len()).is_err() {
                self.failed = true;
                return Err(io::Error::other("cannot allocate output buffer"));
            }
            buffer.extend_from_slice(bytes);
            self.pending += bytes.len();
            Ok(())
        } else {
            if let Err(error) = self.target.write_all(bytes) {
                self.failed = true;
                return Err(error);
            }
            Ok(())
        }
    }
    pub fn finish(&mut self) -> NativeResult<()> {
        self.check()?;
        while !self.buffers.is_empty() {
            self.check()?;
            self.drain(true)?;
        }
        Ok(())
    }
}
impl<W: Write> Write for BufferedOutput<'_, W> {
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
        self.append(bytes)?;
        self.generated = total;
        Ok(bytes.len())
    }
    // Transport flush deliberately leaves every application buffer untouched.
    fn flush(&mut self) -> io::Result<()> {
        self.check().map_err(io::Error::other)?;
        if let Err(error) = self.target.flush() {
            self.failed = true;
            return Err(error);
        }
        Ok(())
    }
}
impl<W: Write> ScriptOutput for BufferedOutput<'_, W> {
    fn generated_bytes(&self) -> Option<usize> {
        Some(self.generated)
    }
    fn remaining_bytes(&self) -> Option<usize> {
        self.limit.map(|n| n.saturating_sub(self.generated))
    }
    fn control(&mut self, action: OutputAction) -> NativeResult<Value> {
        self.check()?;
        self.value(0)?;
        use OutputAction::*;
        match action {
            Start => {
                if self.buffers.len() >= self.depth {
                    return Err(NativeError::new(
                        "OutputError",
                        "output buffer nesting limit exceeded",
                    ));
                }
                self.buffers.push(vec![]);
                Ok(Value::Null)
            }
            Level => Ok(Value::Int(self.buffers.len() as i64)),
            Length => Ok(self
                .buffers
                .last()
                .map_or(Value::Null, |b| Value::Int(b.len() as i64))),
            Contents | GetClean | GetFlush => {
                let Some(buffer) = self.buffers.last() else {
                    if matches!(action, Contents) {
                        return Ok(Value::Null);
                    }
                    return Err(NativeError::new("OutputError", "no active output buffer"));
                };
                self.value(buffer.len())?;
                let mut result = Vec::new();
                result.try_reserve_exact(buffer.len()).map_err(|_| {
                    NativeError::new("ResourceLimitError", "cannot allocate output result")
                })?;
                result.extend_from_slice(buffer);
                if matches!(action, GetClean) {
                    self.pending -= self.buffers.pop().unwrap().len();
                }
                if matches!(action, GetFlush) {
                    self.drain(true)?;
                }
                Ok(Value::Bytes(result.into()))
            }
            Flush | EndFlush => {
                self.drain(matches!(action, EndFlush))?;
                Ok(Value::Null)
            }
            Clean | EndClean => {
                let Some(buffer) = self.buffers.last_mut() else {
                    return Err(NativeError::new("OutputError", "no active output buffer"));
                };
                self.pending -= buffer.len();
                *buffer = Vec::new();
                if matches!(action, EndClean) {
                    self.buffers.pop();
                }
                Ok(Value::Null)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_retrieval_does_not_consume_buffer_and_cancellation_discards_pending_work() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut target = vec![];
        let mut options = RuntimeOptions {
            max_output_buffer_bytes: 4,
            max_value_bytes: Some(size_of::<Value>() + 3),
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        };
        let mut output = BufferedOutput::new(&mut target, &options).unwrap();
        output.control(OutputAction::Start).unwrap();
        output.write_all(b"1234").unwrap();
        assert_eq!(
            output.control(OutputAction::GetClean).unwrap_err().kind,
            "ResourceLimitError"
        );
        assert_eq!(output.control(OutputAction::Length).unwrap(), Value::Int(4));
        flag.store(true, Ordering::Release);
        assert_eq!(output.finish().unwrap_err().kind, "Interrupted");
        drop(output);
        assert!(target.is_empty());
        options.interrupt_flag = None;
        options.max_value_bytes = Some(size_of::<Value>() + 4);
        let mut output = BufferedOutput::new(&mut target, &options).unwrap();
        output.control(OutputAction::Start).unwrap();
        output.write_all(b"1234").unwrap();
        assert_eq!(
            output.control(OutputAction::GetClean).unwrap(),
            Value::Bytes(b"1234".to_vec().into())
        );
    }
}
