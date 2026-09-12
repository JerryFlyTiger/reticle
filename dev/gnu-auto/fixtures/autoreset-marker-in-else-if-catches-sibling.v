module r8 (input clk, input rst_n, input x);
   reg a, b, c;
   always @(posedge clk or negedge rst_n) begin
      if (x) begin
         a <= 1'b1;
      end
      else if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         b <= 1'b1;
         c <= 1'b1;
      end
   end
endmodule
