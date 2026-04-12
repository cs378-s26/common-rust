(zucchini-register
 [g] :project
 (select-window
  (get-buffer-window
   (compile
    (string-join
     '("cargo"
       "buildtool"
       "qemu-test"
       "test_cfgs/virtual_memory/virtual_memory_x86_64_test.json"
       "--stdout")
     " ")))))
