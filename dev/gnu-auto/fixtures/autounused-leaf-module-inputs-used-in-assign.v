module u5 (input clk, input [7:0] a_i, input [3:0] spare_i, output [7:0] z_o);
   assign z_o = a_i + {4'b0, spare_i};
   wire _unused_ok = &{1'b0,
                       /*AUTOUNUSED*/
                       1'b0};
endmodule
