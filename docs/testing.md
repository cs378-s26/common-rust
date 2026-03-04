# Testing
## Unittests
Our unittests are all inline. 
Marking a function with `#[test_case]` in the `kernel_common` lib will let it get run via `cargo buildtool test`.

## Integration Tests
New integration tests can be defined by:
1. Writing a kernel test binary crate in `tests/`
2. Writing a testing configuration json in `test_cfgs`

### Config Json Fields
Config json is responsible for:
1. Pointing test runner to test crate
2. Outlining Qemu Fields
3. Pointing to expected output
4. Defining the number of runs needed to be successful
5. Defining a per-run QEMU timeout in milliseconds

Full model is below
```typescript
type TestConfig = {
    // for all integration tests, this value is false
    is_unittest: boolean,
    n_runs: int,
    timeout_ms: int, // required, must be >= 1
    test_name?: string,
    expected_output_path: string,
    target: "x86_64" | "aarch64",
    qemu_args: string[]
    
}
```

The build tool will recursively search all directories in `test_cfgs` for files matching the pattern `test_cfgs/**/*_test.json`, and run these as tests.
Any `qemu_args` sub string matching the pattern `{PATH_TO_EFI}` and `{PATH_TO_IMG}` will get substituted with the corresponding path.

An example unittest can be found in `tests/example_integration.rs` with the corresponding config json in `test_cfgs/example_integration/example_x86_64_test.json`.
