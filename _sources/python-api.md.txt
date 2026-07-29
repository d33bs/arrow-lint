# Python API

The Python API exposes the same Rust engine used by the command line interface.
Functions return plain Python data structures or rendered report strings.

```python
from arrow_lint import diff, formats, lint, render, render_diff, rules

report = lint("dataset", config=".arrowlint.yaml")
text = render("dataset", output="text")
focused_report = lint("dataset", only=["AL011", "AL014"])
focused_text = render("dataset", only=["AL011"], disabled=["AL014"])
changes = diff("old.parquet", "new.parquet")
diff_text = render_diff("old.parquet", "new.parquet")
all_rules = rules()
known_formats = formats()
```

The `only` and `disabled` keyword arguments accept rule IDs. When `only` is
provided, it replaces the configuration file's `rules.only` value for that
call. `disabled` values are added to configured exclusions, and exclusions
always take precedence.

```{eval-rst}
arrow_lint.api
-------------------
.. automodule:: arrow_lint.api
   :members:
   :private-members:
   :undoc-members:
   :show-inheritance:
```
