module p4 #(parameter WIDTH = 8) (input clk, input rst_n);
   logic [WIDTH-1:0][7:0] arr;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else arr <= '0;
   end
endmodule
