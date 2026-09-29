# Configuration file for the Sphinx documentation builder.

import datetime
import os
from typing import List

# -- Project information -----------------------------------------------------

repository = "cim"
project = "Code in Motion"
year_now = datetime.datetime.now().year
copyright = f"2024-{year_now}, Analog Devices, Inc"
author = "Travis Collins"

version = "1.1.4"
release = version

# -- General configuration ---------------------------------------------------

extensions = [
    "myst_parser",
    "adi_doctools",
    "sphinxcontrib.mermaid",
]

# -- Mermaid configuration ---------------------------------------------------

mermaid_init_js = """
mermaid.initialize({
    startOnLoad: true,
    theme: "base",
    themeVariables: {
        primaryColor: "#0067b9",
        primaryTextColor: "#ffffff",
        primaryBorderColor: "#00305b",
        secondaryColor: "#f0f1f3",
        secondaryTextColor: "#101820",
        secondaryBorderColor: "#659ad2",
        tertiaryColor: "#e5e5e5",
        tertiaryTextColor: "#101820",
        tertiaryBorderColor: "#ccc",
        lineColor: "#00305b",
        textColor: "#101820",
        fontFamily: "Segoe UI, Helvetica Neue, Arial, sans-serif",
        fontSize: "14px",
        noteTextColor: "#101820",
        noteBkgColor: "#f0f1f3",
        noteBorderColor: "#659ad2"
    }
});
"""

templates_path = ["_templates"]

exclude_patterns: List[str] = []

source_suffix = {
    ".rst": "restructuredtext",
    ".md": "markdown",
}

# -- Options for HTML output -------------------------------------------------

html_theme = "cosmic"
html_favicon = os.path.join("_static", "favicon.svg")

html_static_path = ["_static"]

html_css_files = [
    "css/style.css",
]

html_theme_options = {
    "light_logo": os.path.join("logos", "CIM_Logo_300.svg"),
    "dark_logo": os.path.join("logos", "CIM_Logo_w_300.svg"),
}
