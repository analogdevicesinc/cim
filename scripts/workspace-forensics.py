#!/usr/bin/env python3
# Copyright (c) 2026 Analog Devices, Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""
workspace-forensics.py - Trace workspace origin from .workspace marker

Given a workspace marker file, finds the exact cim-manifests
commit that was used to create the workspace.

Usage:
    workspace-forensics.py <workspace-path> [--manifests <path-or-url>]
    workspace-forensics.py /path/to/workspace
    workspace-forensics.py /path/to/.workspace --manifests ~/devel/cim-manifests
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime
from pathlib import Path
from typing import Optional, Dict, List, Tuple

try:
    import yaml
except ImportError:
    print("Error: PyYAML is required. Install with: pip install pyyaml", file=sys.stderr)
    sys.exit(1)


# ANSI color codes for output
class Colors:
    HEADER = '\033[95m'
    OKBLUE = '\033[94m'
    OKCYAN = '\033[96m'
    OKGREEN = '\033[92m'
    WARNING = '\033[93m'
    FAIL = '\033[91m'
    ENDC = '\033[0m'
    BOLD = '\033[1m'
    UNDERLINE = '\033[4m'

    @classmethod
    def disable(cls):
        cls.HEADER = ''
        cls.OKBLUE = ''
        cls.OKCYAN = ''
        cls.OKGREEN = ''
        cls.WARNING = ''
        cls.FAIL = ''
        cls.ENDC = ''
        cls.BOLD = ''
        cls.UNDERLINE = ''


def print_header(text: str):
    """Print a section header"""
    print(f"\n{Colors.BOLD}{Colors.HEADER}{'=' * 70}{Colors.ENDC}")
    print(f"{Colors.BOLD}{Colors.HEADER}{text}{Colors.ENDC}")
    print(f"{Colors.BOLD}{Colors.HEADER}{'=' * 70}{Colors.ENDC}")


def print_info(label: str, value: str, indent: int = 0):
    """Print labeled information"""
    prefix = "  " * indent
    print(f"{prefix}{Colors.OKBLUE}{label}:{Colors.ENDC} {value}")


def print_success(text: str):
    """Print success message"""
    print(f"{Colors.OKGREEN}✓ {text}{Colors.ENDC}")


def print_warning(text: str):
    """Print warning message"""
    print(f"{Colors.WARNING}⚠ {text}{Colors.ENDC}")


def print_error(text: str):
    """Print error message"""
    print(f"{Colors.FAIL}✗ {text}{Colors.ENDC}")


def read_workspace_marker(marker_path: Path) -> Dict:
    """Read and parse .workspace YAML file"""
    try:
        with open(marker_path, 'r') as f:
            data = yaml.safe_load(f)
        return data
    except Exception as e:
        print_error(f"Failed to read workspace marker: {e}")
        sys.exit(1)


def compute_file_sha256(file_path: Path) -> str:
    """Compute SHA256 hash of a file"""
    sha256_hash = hashlib.sha256()
    with open(file_path, "rb") as f:
        for byte_block in iter(lambda: f.read(4096), b""):
            sha256_hash.update(byte_block)
    return sha256_hash.hexdigest()


def is_valid_workspace(marker_path: Path) -> bool:
    """Check if marker_path is in a valid workspace

    A valid workspace has:
    - .workspace marker file
    - sdk.yml file in the same directory
    """
    workspace_dir = marker_path.parent if marker_path.is_file() else marker_path
    sdk_yml = workspace_dir / 'sdk.yml'
    return sdk_yml.exists()


def check_workspace_config_drift(marker_path: Path, marker_data: Dict) -> Optional[Dict]:
    """Check if workspace sdk.yml differs from the original

    Returns dict with drift information or None if no drift or not in workspace
    {
        'current_sha256': str,
        'original_sha256': str,
        'has_drift': bool
    }
    """
    workspace_dir = marker_path.parent if marker_path.is_file() else marker_path
    sdk_yml = workspace_dir / 'sdk.yml'

    if not sdk_yml.exists():
        return None

    original_sha256 = marker_data.get('config_sha256')
    if not original_sha256:
        return None

    current_sha256 = compute_file_sha256(sdk_yml)
    has_drift = (current_sha256 != original_sha256)

    return {
        'current_sha256': current_sha256,
        'original_sha256': original_sha256,
        'has_drift': has_drift
    }


def run_git_command(args: List[str], cwd: Path, capture_output: bool = True, silent_errors: bool = False) -> subprocess.CompletedProcess:
    """Run a git command in the specified directory

    Args:
        args: Git command arguments
        cwd: Working directory
        capture_output: Whether to capture stdout/stderr
        silent_errors: If True, don't print error messages (useful for expected failures)
    """
    try:
        result = subprocess.run(
            ['git'] + args,
            cwd=cwd,
            capture_output=capture_output,
            text=True,
            check=True
        )
        return result
    except subprocess.CalledProcessError as e:
        if not capture_output:
            raise
        if not silent_errors:
            print_error(f"Git command failed: {' '.join(args)}")
            print_error(f"Error: {e.stderr}")
        raise


def clone_manifests_repo(source: str, verbose: bool = False) -> Path:
    """Clone manifests repository to temporary directory"""
    temp_dir = Path(tempfile.mkdtemp(prefix="workspace-forensics-"))

    if verbose:
        print_info("Cloning manifests", f"{source} -> {temp_dir}")
    else:
        print("Cloning manifests repository...")

    try:
        # Clone with full history
        subprocess.run(
            ['git', 'clone', source, str(temp_dir)],
            capture_output=not verbose,
            text=True,
            check=True
        )

        # Fetch all branches
        run_git_command(['fetch', '--all'], temp_dir, capture_output=not verbose)

        if verbose:
            print_success(f"Cloned to {temp_dir}")

        return temp_dir
    except subprocess.CalledProcessError as e:
        print_error(f"Failed to clone repository: {e}")
        sys.exit(1)


def get_all_branches(repo_path: Path, verbose: bool = False) -> List[str]:
    """Get all remote branches

    Returns branches in format 'remote/branch' (e.g., 'origin/main', 'upstream/dev')
    """
    if verbose:
        print("\nFetching all branches...")

    result = run_git_command(['branch', '-r'], repo_path)
    # Keep full remote/branch format, filter out HEAD references
    branches = [b.strip() for b in result.stdout.split('\n') if b.strip() and '->' not in b]

    if verbose:
        print(f"  Found {len(branches)} branches across all remotes")

    return branches


def find_branches_with_target(repo_path: Path, target: str, verbose: bool = False) -> List[str]:
    """Find all branches that contain the target directory at HEAD

    NOTE: This only checks current HEAD. A branch may have had the target
    in the past but deleted it later, and it won't be found here.
    This is used for display purposes, not for searching commits.

    Returns branches in format 'remote/branch'
    """
    branches = []
    target_path = f"targets/{target}/sdk.yml"

    if verbose:
        print(f"\nSearching for target '{target}' at branch HEADs...")

    # Get all remote branches (already in remote/branch format)
    all_branches = get_all_branches(repo_path, verbose=False)

    for branch in all_branches:
        # Check if target exists at HEAD of this branch
        try:
            run_git_command(['show', f'{branch}:{target_path}'], repo_path, silent_errors=True)
            branches.append(branch)
            if verbose:
                print_success(f"Found in branch HEAD: {branch}")
        except subprocess.CalledProcessError:
            # Expected - target doesn't exist in this branch
            continue

    return branches


def format_timestamp(timestamp_str: str) -> str:
    """Format Unix timestamp to human-readable date"""
    try:
        ts = int(timestamp_str)
        dt = datetime.fromtimestamp(ts)
        return dt.strftime('%Y-%m-%d %H:%M:%S')
    except:
        return timestamp_str


def find_commit_by_sha256(
    repo_path: Path,
    target: str,
    expected_sha256: str,
    verbose: bool = False,
    quiet: bool = False
) -> Optional[Dict]:
    """
    Find the commit where sdk.yml matches the expected SHA256.

    Searches ALL branches, even if the target was deleted from HEAD later.
    This ensures we find historical matches.

    Returns: Dict with match details or None
    {
        'branch': str,
        'commit': Dict (commit info with hash, author, email, timestamp, subject),
        'is_current': bool (does HEAD have this config?)
    }
    """
    target_path = f"targets/{target}/sdk.yml"

    if not quiet and verbose:
        print(f"\nSearching for config with SHA256: {expected_sha256[:16]}...")

    # Get ALL branches - don't filter by whether target exists at HEAD
    branches = get_all_branches(repo_path, verbose=False)

    for branch in branches:
        if verbose:
            print(f"\n  Checking branch: {branch}")

        # Get list of commits that modified the target file
        try:
            result = run_git_command(
                ['log', '--format=%H', f'{branch}', '--', target_path],
                repo_path,
                silent_errors=True
            )
            commits = [c.strip() for c in result.stdout.split('\n') if c.strip()]

            if verbose:
                print(f"    Found {len(commits)} commits modifying {target_path}")

            # Check each commit
            for i, commit in enumerate(commits):
                try:
                    # Get file content at this commit
                    result = run_git_command(['show', f'{commit}:{target_path}'], repo_path, silent_errors=True)
                    content = result.stdout

                    # Compute SHA256
                    sha256 = hashlib.sha256(content.encode()).hexdigest()

                    if sha256 == expected_sha256:
                        # Get commit details
                        result = run_git_command(
                            ['show', '--format=%H%n%an%n%ae%n%at%n%s', '--no-patch', commit],
                            repo_path
                        )
                        lines = result.stdout.strip().split('\n')

                        commit_info = {
                            'hash': lines[0],
                            'author': lines[1],
                            'email': lines[2],
                            'timestamp': lines[3],
                            'subject': lines[4] if len(lines) > 4 else '',
                        }

                        # Check if current HEAD has this config
                        is_current = False
                        try:
                            result = run_git_command(['show', f'{branch}:{target_path}'], repo_path, silent_errors=True)
                            head_sha256 = hashlib.sha256(result.stdout.encode()).hexdigest()
                            is_current = (head_sha256 == expected_sha256)
                        except subprocess.CalledProcessError:
                            pass

                        if verbose:
                            print_success(f"Found matching commit in branch '{branch}'!")

                        return {
                            'branch': branch,
                            'commit': commit_info,
                            'is_current': is_current
                        }

                except subprocess.CalledProcessError:
                    continue

        except subprocess.CalledProcessError:
            if verbose:
                print_warning(f"Could not get commit history for branch {branch}")
            continue

    return None


def find_tags_for_commit(repo_path: Path, commit_hash: str) -> List[str]:
    """Find all tags that point to a specific commit"""
    try:
        result = run_git_command(['tag', '--points-at', commit_hash], repo_path)
        tags = [t.strip() for t in result.stdout.split('\n') if t.strip()]
        return tags
    except subprocess.CalledProcessError:
        return []


def should_show_remotes(branches: List[str]) -> bool:
    """Determine if we should show remote names in output

    Returns True if branches come from multiple remotes
    """
    remotes = set()
    for branch in branches:
        if '/' in branch:
            remote = branch.split('/', 1)[0]
            remotes.add(remote)
    return len(remotes) > 1


def format_branch_for_display(branch: str, show_remote: bool) -> str:
    """Format branch name for display

    Args:
        branch: Full branch name in 'remote/branch' format
        show_remote: Whether to include remote name in output

    Returns:
        Formatted branch name
    """
    if '/' not in branch:
        return branch

    if show_remote:
        # Show as 'branch (remote)' for clarity
        remote, branch_name = branch.split('/', 1)
        return f"{branch_name} ({remote})"
    else:
        # Just the branch name
        return branch.split('/', 1)[1]


def display_results(
    marker_data: Dict,
    branches_with_target: List[str],
    match_result: Optional[Dict],
    marker_path: Path,
    config_drift: Optional[Dict] = None
):
    """Display forensics results"""

    # Workspace marker information
    print("📋 Workspace Marker Information:")
    print_info("Marker file", str(marker_path), 1)
    print_info("Target", marker_data.get('target') or marker_data.get('original_config_file', 'unknown'), 1)
    print_info("Target version", marker_data.get('target_version', 'unknown'), 1)
    print_info("Created at", format_timestamp(marker_data.get('created_at', '0')), 1)
    print_info("Config SHA256", marker_data.get('config_sha256', 'unknown')[:16] + "...", 1)
    print_info("Mirror path", marker_data.get('mirror_path', 'unknown'), 1)

    # Check for config drift in workspace
    if config_drift and config_drift['has_drift']:
        print()
        print_warning("  ⚠ Workspace config has been modified!")
        print_info("Current SHA256", config_drift['current_sha256'][:16] + "...", 1)
        print_warning("  The sdk.yml in this workspace differs from the original used during init")

    print("\n🔧 SDK Manager Version:")
    print_info("Version", marker_data.get('sdk_manager_version', 'unknown'), 1)
    print_info("Commit", marker_data.get('sdk_manager_commit', 'unknown'), 1)
    print_info("SHA256", marker_data.get('sdk_manager_sha256', 'unknown')[:16] + "...", 1)

    # Target availability
    print("\n🎯 Target Availability:")
    if branches_with_target:
        target = marker_data.get('target') or marker_data.get('original_config_file', 'unknown')
        target_version = marker_data.get('target_version', '')

        # Categorize branches
        primary_branches = []
        other_branches = []

        for branch in branches_with_target:
            # Extract branch name without remote prefix for matching
            branch_name = branch.split('/', 1)[1] if '/' in branch else branch

            # Check if branch name matches target or target_version
            if branch_name == target_version:
                primary_branches.append((branch, 'exact'))
            elif target in branch_name:
                # Only consider it related if it actually contains the target name
                primary_branches.append((branch, 'related'))
            else:
                other_branches.append(branch)

        # Determine if we should show remote names
        show_remotes = should_show_remotes(branches_with_target)

        # Display primary branches
        if primary_branches:
            print_info("Primary branches (matching target/version)", "", 1)
            for branch, match_type in primary_branches:
                display_branch = format_branch_for_display(branch, show_remotes)
                if match_type == 'exact':
                    print(f"    • {display_branch} {Colors.OKGREEN}(exact version match) ✓{Colors.ENDC}")
                else:
                    print(f"    • {display_branch} {Colors.OKCYAN}(related){Colors.ENDC}")

        # Display other branches
        if other_branches:
            if primary_branches:
                print()
            print_info("Other branches with this target", "", 1)
            for branch in other_branches:
                display_branch = format_branch_for_display(branch, show_remotes)
                print(f"    • {display_branch}")
    else:
        print_warning(f"  Target '{marker_data.get('target')}' not found in any branch")

    # Exact commit match
    print("\n🔍 Configuration Match:")
    if match_result:
        branch = match_result['branch']
        commit = match_result['commit']
        is_current = match_result['is_current']

        # Determine if we should show remote (check all branches context)
        all_branches = branches_with_target + ([branch] if branch not in branches_with_target else [])
        show_remotes = should_show_remotes(all_branches)
        display_branch = format_branch_for_display(branch, show_remotes)

        print_success(f"  Found exact match!")
        print_info("Commit", commit['hash'][:12], 1)
        print_info("Branch", display_branch, 1)
        print_info("Author", f"{commit['author']} <{commit['email']}>", 1)
        print_info("Date", format_timestamp(commit['timestamp']), 1)
        print_info("Message", commit['subject'], 1)

        if is_current:
            status = f"Branch '{display_branch}' still has this exact config at HEAD"
        else:
            status = f"Branch '{display_branch}' has been updated (HEAD has different config)"
        print_info("Status", status, 1)

        print("\n📝 Reproduction Commands:")
        print(f"  git checkout {commit['hash']}")
        print(f"  # Or use branch: git checkout {display_branch}")
        print(f"  # Target location: targets/{marker_data.get('target') or marker_data.get('original_config_file', 'unknown')}/sdk.yml")
    else:
        print_error("  No matching commit found")
        print_warning("  Possible reasons:")
        print("    • Config file was modified locally after init")
        print("    • Manifest repository is incomplete")
        print("    • Config was generated from a different source")


def main():
    parser = argparse.ArgumentParser(
        description="Trace workspace origin from .workspace marker file",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s /path/to/workspace
  %(prog)s /path/to/.workspace
  %(prog)s ~.workspace --manifests ~/devel/cim-manifests
  %(prog)s ~.workspace --manifests https://github.com/joabech/cim-manifests
  %(prog)s ~.workspace --json > report.json
        """
    )

    parser.add_argument(
        'workspace_path',
        type=Path,
        help='Path to workspace directory or .workspace file'
    )

    parser.add_argument(
        '--manifests',
        '-m',
        type=str,
        default=os.path.expanduser('~/devel/cim-manifests'),
        help='Path or URL to cim-manifests repository (default: ~/devel/cim-manifests)'
    )

    parser.add_argument(
        '--json',
        action='store_true',
        help='Output results as JSON'
    )

    parser.add_argument(
        '--verbose',
        '-v',
        action='store_true',
        help='Enable verbose output'
    )

    parser.add_argument(
        '--no-color',
        action='store_true',
        help='Disable colored output'
    )

    args = parser.parse_args()

    if args.no_color or args.json:
        Colors.disable()

    # Determine marker file path
    workspace_path = args.workspace_path.resolve()
    if workspace_path.is_dir():
        marker_path = workspace_path / '.workspace'
    else:
        marker_path = workspace_path

    if not marker_path.exists():
        print_error(f"Workspace marker not found: {marker_path}")
        sys.exit(1)

    # Read workspace marker
    marker_data = read_workspace_marker(marker_path)

    if args.verbose:
        print("\nMarker data:")
        print(json.dumps(marker_data, indent=2))

    # Support both new 'target' field and legacy 'original_config_file' field
    target = marker_data.get('target') or marker_data.get('original_config_file')
    if not target:
        print_error("Target not found in workspace marker (neither 'target' nor 'original_config_file')")
        sys.exit(1)

    config_sha256 = marker_data.get('config_sha256')
    if not config_sha256:
        print_error("Config SHA256 not found in workspace marker")
        sys.exit(1)

    # Clone manifests repository
    repo_path = None
    temp_repo = False

    # Check if manifests path is a local git repo
    manifests_path = Path(args.manifests).resolve()
    if manifests_path.exists() and (manifests_path / '.git').exists():
        repo_path = manifests_path
        if args.verbose:
            print_info("Using local repository", str(repo_path))
    else:
        # Need to clone
        repo_path = clone_manifests_repo(args.manifests, args.verbose)
        temp_repo = True

    try:
        # Check for config drift if in a workspace
        config_drift = check_workspace_config_drift(marker_path, marker_data)

        # Find branches containing target at HEAD (for display purposes)
        branches_with_target = find_branches_with_target(repo_path, target, args.verbose)

        # Search for exact commit match across ALL branches (including historical commits)
        # Don't limit to branches_with_target - the config might have been deleted from HEAD
        match_result = find_commit_by_sha256(
            repo_path,
            target,
            config_sha256,
            args.verbose,
            args.json
        )

        # Output results
        if args.json:
            output = {
                'marker_path': str(marker_path),
                'marker_data': marker_data,
                'branches_with_target': branches_with_target,
                'match': None,
                'config_drift': config_drift
            }

            if match_result:
                output['match'] = match_result

            print(json.dumps(output, indent=2))
        else:
            display_results(marker_data, branches_with_target, match_result, marker_path, config_drift)

        # Exit code based on whether we found a match
        sys.exit(0 if match_result else 1)

    finally:
        # Clean up temporary clone
        if temp_repo and repo_path:
            if args.verbose:
                print(f"\nCleaning up temporary clone: {repo_path}")
            import shutil
            shutil.rmtree(repo_path, ignore_errors=True)


if __name__ == '__main__':
    main()
