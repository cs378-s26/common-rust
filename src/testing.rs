#[cfg(test)]
use alloc::string::String;

#[cfg(test)]
use spin::Mutex;

#[cfg(test)]
static ACTUAL: Mutex<String> = Mutex::new(String::new());

pub macro test_output($text:literal) {
    ACTUAL.lock().push_str($text)
}

pub macro unit_test($expected:literal, $code:block) {
    #[cfg(test)]
    #[test_case]
    struct UnitTestGuard {
        expected: String,
    }

    #[cfg(test)]
    #[test_case]
    impl UnitTestGuard {
        fn new(expected: &'static str) -> UnitTestGuard {
            assert!(*ACTUAL.lock() == Into::<String>::into(""));
            UnitTestGuard {
                expected: expected.into(),
            }
        }
    }

    #[cfg(test)]
    #[test_case]
    impl Drop for UnitTestGuard {
        fn drop(&mut self) {
            if { self.expected != *ACTUAL.lock() } {
                panic!(
                    "\nExpected:\n{}\nActual:\n{}",
                    self.expected,
                    ACTUAL.lock()
                );
            }
            *ACTUAL.lock() = "".into()
        }
    }
    #[cfg(test)]
    #[test_case]
    fn unit_test() {
        let _guard = UnitTestGuard::new($expected);
        $code // run test
        drop(_guard) // check output
    }
}

unit_test!("hello unit test world!", {
    test_output!("hello unit test world!");
});
