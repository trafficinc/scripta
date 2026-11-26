use scr_runtime::{NativeError, Runtime, RuntimeOptions};
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn cleanup_runs_after_completion_exit_failure_and_fatal_limits() {
    for (source, options) in [
        ("$x=1;", RuntimeOptions::default()),
        ("exit 7;", RuntimeOptions::default()),
        ("missing();", RuntimeOptions::default()),
        (
            "while (true) {}",
            RuntimeOptions {
                instruction_limit: Some(10),
                ..Default::default()
            },
        ),
        (
            "",
            RuntimeOptions {
                interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
                ..Default::default()
            },
        ),
        (
            "",
            RuntimeOptions {
                max_scopes: Some(0),
                ..Default::default()
            },
        ),
    ] {
        let order = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = Runtime::with_options(options);
        for index in 0..3 {
            let order = order.clone();
            runtime.add_execution_cleanup(move || {
                order.borrow_mut().push(index);
                Ok(())
            });
        }
        let _ = runtime.execute_source(source, &mut Vec::new());
        assert_eq!(*order.borrow(), [2, 1, 0], "{source}");
        order.borrow_mut().clear();
        drop(runtime);
        assert_eq!(*order.borrow(), [2, 1, 0]);
    }
}
#[test]
fn cleanup_errors_preserve_original_failures_and_do_not_skip_other_callbacks() {
    for source in ["$x=1;", "missing();"] {
        let ran = Rc::new(RefCell::new(0));
        let mut runtime = Runtime::new();
        let count = ran.clone();
        runtime.add_execution_cleanup(move || {
            *count.borrow_mut() += 1;
            Ok(())
        });
        runtime.add_execution_cleanup(|| {
            Err(NativeError::new("CleanupError", "injected cleanup failure"))
        });
        let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
        assert!(
            error
                .message
                .contains("CleanupError: injected cleanup failure")
        );
        if source == "missing();" {
            assert!(error.message.contains("missing"));
            assert!(error.message.contains("native cleanup failed"));
            assert_eq!(error.span.start, 0);
        }
        assert_eq!(*ran.borrow(), 1);
    }
}
struct FailingOutput;
impl Write for FailingOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("injected output failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn output_failures_still_run_native_cleanup() {
    let ran = Rc::new(RefCell::new(false));
    let mark = ran.clone();
    let mut runtime = Runtime::new();
    runtime.add_execution_cleanup(move || {
        *mark.borrow_mut() = true;
        Ok(())
    });
    assert!(
        runtime
            .execute_source("echo 'value';", &mut FailingOutput)
            .is_err()
    );
    assert!(*ran.borrow());
}
