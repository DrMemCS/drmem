# Logic Ideas

These are ideas for "logic" nodes. Logic nodes are pure software
instances that monitor device(s) for input, perform a calculation, and
send the result(s) to another device.

- [ ] "Const" node (Out: `output`) Its configuration specifies a value
      that this node always outputs. At boot, this mode will send that
      constant to the attached, settable output device.
- [ ] Inverter node (In: `input`, Out: `output`) Takes a boolean
      device and complements its value.
- [ ] Or/And nodes (In: `input`, Out: `output`) Takes an array of
      boolean devices and sets the output based on the boolean OR/AND
      of all the inputs.
- [ ] Switch node (In: `in_false`, `in_true`, `selection`; Out:
      `result`) The `selection` input determines which input gets
      routed to the output, `result`. `result` could change when
      `selection` switches which input is routed to the output. Also,
      if an input is selected and its value changes, that change is
      propagated. The types of `in_false` and `in_true` should be the
      same.
- [ ] Min/Max node (In: `input`, `reset`; Out: `output`) This node
      remembers the last value of `input` and maintains the max/min
      value. `output` is where the max/min is sent. If `reset` goes
      false to true, `output` is set to the last value. This node's
      `input` and `output` are limited to floating point, integer, and
      boolean values.
- [ ] Sample node (In: `input`, `trigger`; Out: `output`) Every time
      `trigger` goes false to true, `output` is set to the value of
      `input` -- even if the value hasn't changed. This node can turn
      an event-driven device into a periodic one. This node is
      "typeless" in that it will simply copy whatever value is read
      from `input`.
- [ ] Calc node (In: (`A`, `B`, etc.), `expr`; Out: `result`) The
      config parameters define the inputs, `A` through .. and an
      expression, `expr`. As the inputs change, the expression is
      evaluated and its result is written to `result`. The expression
      will determine what types are needed for the inputs and the
      result.
- [ ] Alarm node (In: `value`, `min`, `max`; Out: `alarm`, `high`,
      `low`) Compares its input value to the two limits and sets the
      output devices accordingly. Not all output devices need to be
      defined.
