input_array: dc.l 10, 3, -1, 2, 0, 4, 17, -4, 8
pople_input_array:
input_length: dc.l (pople_input_array-input_array)/4
partial_sums: dcb.l (pople_input_array-input_array)/4,0