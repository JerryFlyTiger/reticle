module r5 (input clk, input rst_n);
   reg a, b, c;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         a <= 1'b1;
         b = 1'b1;
         if (a) c <= 1'b0;
      end
   end
endmodule
