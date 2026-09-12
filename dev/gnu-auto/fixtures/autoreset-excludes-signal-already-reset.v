module r2 (input clk, input rst_n, input [7:0] d, output reg [7:0] q, output reg v, output reg z);
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         z <= 8'hff;
         /*AUTORESET*/
      end
      else begin
         q <= d;
         v <= 1'b1;
         z <= d;
      end
   end
endmodule
