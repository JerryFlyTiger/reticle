module r7 (input clk, input rst_n);
   reg a, b, other;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         a <= 1'b1;
      end
   end
   always @(posedge clk) begin
      other <= 1'b1;
   end
endmodule
