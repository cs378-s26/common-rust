(zucchini-register
 [g] :project
 (select-window
  (get-buffer-window
   (compile
    (string-join
     '("cargo"
       "buildtool"
       "qemu-test"
       "test_cfgs/example_integration/example_aarch64_test.json"
       "--stdout")
     " "))))
 [d] :project (call-interactively #'lldb)
 [j] :project (vterm))
