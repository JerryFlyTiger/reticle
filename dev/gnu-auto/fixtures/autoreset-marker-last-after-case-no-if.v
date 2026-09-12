module r6 (input clk, input rst_n, input [1:0] s);
   reg a, b;
   always @(posedge clk) begin
      case (s)
        2'd0: a <= 1'b1;
        default: b <= 1'b0;
      endcase
      /*AUTORESET*/
   end
endmodule
