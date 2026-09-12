module u1 (input clk, input rst_n, input [7:0] used_i, input [3:0] unused_a_i, input unused_b_i, output reg o);
   always @(posedge clk) o <= used_i[0];
   wire _unused_ok = &{1'b0,
                       /*AUTOUNUSED*/
                       1'b0};
endmodule
