module r1 (input clk, input rst_n, input [7:0] d, output reg [7:0] q, output reg v);
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         q <= d;
         v <= 1'b1;
      end
   end
endmodule
