# ==============================================================================
# Helper targets for ADI HDL guided build flow and inspection
# ==============================================================================

.PHONY: guide list-combos list-projects list-boards list-tools check-tools

guide:
	@bash scripts/build-hdl.sh --interactive

list-combos:
	@bash scripts/build-hdl.sh --list

list-projects:
	@bash scripts/build-hdl.sh --list-projects

list-boards:
	@bash scripts/build-hdl.sh --list-boards $(HDL_PROJECT)

list-tools:
	@bash scripts/build-hdl.sh --list-tools

check-tools:
	@bash scripts/build-hdl.sh --check-tools
