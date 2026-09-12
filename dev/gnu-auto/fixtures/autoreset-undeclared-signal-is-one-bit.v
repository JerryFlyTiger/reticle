module r13 (input clk, input rst_n);
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         undeclared_sig <= 1'b1;
      end
   end
endmodule
