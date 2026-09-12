module r12 (input clk, input rst_n, input [1:0] s);
   reg [7:0] a, b, c, d;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         case (s)
           2'd0: a <= 8'd1;
           2'd1: begin b <= 8'd2; end
           default: c[3:0] <= 4'd3;
         endcase
         for (int i = 0; i < 2; i++) d[i] <= 1'b1;
      end
   end
endmodule
