* The program `cargo run` runs when no file is named on the command line.
* Replace it with whatever you are working on.
    org     $1000
start:
    lea     greeting,a1         ; the string to display
    move.b  #14,d0              ; task 14: display it without a newline
    trap    #15
    move.b  #9,d0               ; task 9: end the program
    trap    #15
greeting:
    dc.b    'hello from s68k',0
    end     start
