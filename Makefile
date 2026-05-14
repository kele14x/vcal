.PHONY: four-value add-op

four-value:
	iverilog -o /tmp/four_value_ops_tb.out doc/four_value_ops_tb.v
	vvp /tmp/four_value_ops_tb.out
