module u4 (input clk, input [7:0] a_i, input [3:0] spare_i, output [7:0] z_o);
   sub u_sub (.clk(clk), .a_i(a_i), .z_o(z_o));
   wire _unused_ok = &{1'b0,
                       /*AUTOUNUSED*/
                       1'b0};
endmodule
