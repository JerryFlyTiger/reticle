module r14 (input logic clk_i, input logic rst_ni, input logic [7:0] d_i, output logic [7:0] q_o);
   logic [3:0] cnt_q;
   always_ff @(posedge clk_i or negedge rst_ni) begin
      if (!rst_ni) begin
         /*AUTORESET*/
      end else begin
         q_o   <= d_i;
         cnt_q <= cnt_q + 4'd1;
      end
   end
endmodule
