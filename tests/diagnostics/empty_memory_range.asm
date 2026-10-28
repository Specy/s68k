* A range whose start is above its end covers no address. EASy68K accepts
* it, so it is a warning and the program still builds.
    memory read $9000,$8000
    nop
