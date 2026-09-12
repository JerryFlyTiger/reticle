module u3 (input clk, input [7:0] a_i, input [3:0] spare_i, inout wire io_b, output [7:0] z_o, output reg r_o);
   always @(posedge clk) r_o <= a_i[0];
   sub u_sub (/*AUTOINST*/);
   wire _unused_ok = &{1'b0,
                       /*AUTOUNUSED*/
                       1'b0};
endmodule
