# Sourced by the release scripts (bash). Loads scripts/release.env when it exists (start from
# scripts/release.env.example), and offers `require_settings NAME...`, which stops with a clear message naming every
# setting that is missing.

release_env_file="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release.env"
if [[ -f "$release_env_file" ]]; then
    # shellcheck source=/dev/null
    source "$release_env_file"
fi

require_settings() {
    local name missing=()
    for name in "$@"; do
        [[ -n "${!name:-}" ]] || missing+=("$name")
    done
    if ((${#missing[@]} > 0)); then
        {
            echo "missing release settings: ${missing[*]}"
            echo "export them, or set them in scripts/release.env (copy scripts/release.env.example and fill it in)"
        } >&2
        exit 1
    fi
}
