* An `offset` region lays out a table of offsets: "no machine code is generated
* by instructions or directives following an OFFSET directive"
* (Directives/offset.htm). `ds` is what the table is made of and says nothing.
        offset  0
field1  ds.w    1
field2  ds.b    2
* An instruction inside the region would have been assembled nowhere.
        move.l  #1,d0
* So would the bytes of a `dc`.
message dc.b    'lost',0
* `org *` ends the region and restores the address in use before it.
        org     *
        move.l  #1,d0
