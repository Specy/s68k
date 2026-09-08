* `include` and `incbin` read a file of this project, and none of the three
* lines below names one. The project holds `lib/io.m68k` and `sprite.bin`.
    include io.m68k
    incbin  data/pixels.bin
    include sprite.bin
* The fourth shape of this diagnostic is the entry file itself, missing or
* holding bytes, which no file of a project can show: `include.rs` has it as
* `an_entry_file_that_is_missing_or_binary_is_the_one_failure`.
    nop
