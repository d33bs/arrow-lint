# Python API

The Python API exposes the same Rust engine used by the command line interface.
Functions return plain Python data structures or rendered report strings.

```python
from arrow_lint import formats, lint, render, rules

report = lint("dataset", config=".arrowlint.yaml")
text = render("dataset", output="text")
all_rules = rules()
known_formats = formats()
```

```{eval-rst}
arrow_lint.api
-------------------
.. automodule:: arrow_lint.api
   :members:
   :private-members:
   :undoc-members:
   :show-inheritance:
```
