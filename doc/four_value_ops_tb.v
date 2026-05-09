`timescale 1 ps / 1 ps
//
`default_nettype none

module four_value_ops_tb;
  localparam integer UNARY_OP_COUNT = 10;
  localparam integer PAIR_OP_COUNT = 24;

  integer output_fd;
  integer i;
  integer op_index;
  reg [3:0] set;

  task automatic write_unary_label;
    input integer op;
    begin
      case (op)
        0: $fwrite(output_fd, "+");
        1: $fwrite(output_fd, "-");
        2: $fwrite(output_fd, "!");
        3: $fwrite(output_fd, "~");
        4: $fwrite(output_fd, "&");
        5: $fwrite(output_fd, "~&");
        6: $fwrite(output_fd, "|");
        7: $fwrite(output_fd, "~|");
        8: $fwrite(output_fd, "^");
        9: $fwrite(output_fd, "~^");
      endcase
    end
  endtask

  task automatic write_pair_label;
    input integer op;
    begin
      case (op)
        0: $fwrite(output_fd, "+");
        1: $fwrite(output_fd, "-");
        2: $fwrite(output_fd, "*");
        3: $fwrite(output_fd, "/");
        4: $fwrite(output_fd, "**");
        5: $fwrite(output_fd, "%%");
        6: $fwrite(output_fd, ">");
        7: $fwrite(output_fd, ">=");
        8: $fwrite(output_fd, "<");
        9: $fwrite(output_fd, "<=");
        10: $fwrite(output_fd, "&&");
        11: $fwrite(output_fd, "||");
        12: $fwrite(output_fd, "==");
        13: $fwrite(output_fd, "!=");
        14: $fwrite(output_fd, "===");
        15: $fwrite(output_fd, "!==");
        16: $fwrite(output_fd, "&");
        17: $fwrite(output_fd, "|");
        18: $fwrite(output_fd, "^");
        19: $fwrite(output_fd, "^~");
        20: $fwrite(output_fd, "<<");
        21: $fwrite(output_fd, ">>");
        22: $fwrite(output_fd, "<<<");
        23: $fwrite(output_fd, ">>>");
      endcase
    end
  endtask

  task automatic write_unary_value;
    input integer op;
    input reg value;
    begin
      case (op)
        0: $fwrite(output_fd, "%b", +value);
        1: $fwrite(output_fd, "%b", -value);
        2: $fwrite(output_fd, "%b", !value);
        3: $fwrite(output_fd, "%b", ~value);
        4: $fwrite(output_fd, "%b", &value);
        5: $fwrite(output_fd, "%b", ~&value);
        6: $fwrite(output_fd, "%b", |value);
        7: $fwrite(output_fd, "%b", ~|value);
        8: $fwrite(output_fd, "%b", ^value);
        9: $fwrite(output_fd, "%b", ~^value);
      endcase
    end
  endtask

  task automatic write_pair_value;
    input integer op;
    input reg lhs;
    input reg rhs;
    begin
      case (op)
        0: $fwrite(output_fd, "%b", lhs + rhs);
        1: $fwrite(output_fd, "%b", lhs - rhs);
        2: $fwrite(output_fd, "%b", lhs * rhs);
        3: $fwrite(output_fd, "%b", lhs / rhs);
        4: $fwrite(output_fd, "%b", lhs ** rhs);
        5: $fwrite(output_fd, "%b", lhs % rhs);
        6: $fwrite(output_fd, "%b", lhs > rhs);
        7: $fwrite(output_fd, "%b", lhs >= rhs);
        8: $fwrite(output_fd, "%b", lhs < rhs);
        9: $fwrite(output_fd, "%b", lhs <= rhs);
        10: $fwrite(output_fd, "%b", lhs && rhs);
        11: $fwrite(output_fd, "%b", lhs || rhs);
        12: $fwrite(output_fd, "%b", lhs == rhs);
        13: $fwrite(output_fd, "%b", lhs != rhs);
        14: $fwrite(output_fd, "%b", lhs === rhs);
        15: $fwrite(output_fd, "%b", lhs !== rhs);
        16: $fwrite(output_fd, "%b", lhs & rhs);
        17: $fwrite(output_fd, "%b", lhs | rhs);
        18: $fwrite(output_fd, "%b", lhs ^ rhs);
        19: $fwrite(output_fd, "%b", lhs ^~ rhs);
        20: $fwrite(output_fd, "%b", lhs << rhs);
        21: $fwrite(output_fd, "%b", lhs >> rhs);
        22: $fwrite(output_fd, "%b", lhs <<< rhs);
        23: $fwrite(output_fd, "%b", lhs >>> rhs);
      endcase
    end
  endtask

  task automatic write_label_padding;
    input integer width;
    input integer op_width;
    integer j;
    begin
      for (j = op_width; j < width; j = j + 1) begin
        $fwrite(output_fd, " ");
      end
    end
  endtask

  task automatic write_rule;
    input integer width;
    integer j;
    begin
      for (j = 0; j < width; j = j + 1) begin
        $fwrite(output_fd, "-");
      end
    end
  endtask

  task automatic write_unary_table;
    input integer op;
    begin
      write_unary_label(op);
      case (op)
        5, 7, 9: write_label_padding(3, 2);
        default: write_label_padding(3, 1);
      endcase
      $fwrite(output_fd, "|\n");

      write_rule(3);
      $fwrite(output_fd, "+---\n");

      for (i = 0; i < 4; i = i + 1) begin
        $fwrite(output_fd, " %b | ", set[i]);
        write_unary_value(op, set[i]);
        $fwrite(output_fd, "\n");
      end

      if (op != UNARY_OP_COUNT - 1) begin
        $fwrite(output_fd, "\n");
      end
    end
  endtask

  task automatic write_pair_table;
    input integer op;
    begin
      write_pair_label(op);
      case (op)
        4, 7, 9, 10, 11, 12, 13, 19, 20, 21: write_label_padding(3, 2);
        14, 15, 22, 23: write_label_padding(3, 3);
        default: write_label_padding(3, 1);
      endcase
      $fwrite(output_fd, "| 0 | 1 | x | z\n");

      write_rule(3);
      $fwrite(output_fd, "+---+---+---+---\n");

      for (i = 0; i < 4; i = i + 1) begin
        $fwrite(output_fd, " %b | ", set[i]);
        write_pair_value(op, set[i], 1'b0);
        $fwrite(output_fd, " | ");
        write_pair_value(op, set[i], 1'b1);
        $fwrite(output_fd, " | ");
        write_pair_value(op, set[i], 1'bx);
        $fwrite(output_fd, " | ");
        write_pair_value(op, set[i], 1'bz);
        $fwrite(output_fd, "\n");
      end

      if (op != PAIR_OP_COUNT - 1) begin
        $fwrite(output_fd, "\n");
      end
    end
  endtask

  initial begin
    set = 4'bzx10;
    output_fd = $fopen("four_value_ops_output.txt", "w");
    if (output_fd == 0) begin
      $display("failed to open four_value_ops_output.txt");
      $finish;
    end

    $fwrite(output_fd, "Unary operators\n");
    for (op_index = 0; op_index < UNARY_OP_COUNT; op_index = op_index + 1) begin
      write_unary_table(op_index);
    end

    $fwrite(output_fd, "\nBinary operators\n");
    for (op_index = 0; op_index < PAIR_OP_COUNT; op_index = op_index + 1) begin
      write_pair_table(op_index);
    end

    $fclose(output_fd);
    $finish;
  end
endmodule

`default_nettype wire
