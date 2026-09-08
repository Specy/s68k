* `page` writes a page break in the list file and names no address, so it takes
* no label ("No label is permitted", Directives/page.htm).
heading page
* The conditional-assembly directives take none either ("IFxx and ENDC
* directives may not be labeled", Directives/conditional.htm). The line is
* refused as well, because conditional assembly is not implemented; the two are
* separate mistakes in separate places.
skip    ifeq 1
        endc
