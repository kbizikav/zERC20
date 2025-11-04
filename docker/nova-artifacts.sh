#!/usr/bin/env bash

prepare_nova_artifacts() {
    local artifacts_dir="${ARTIFACTS_DIR:-/app/nova_artifacts}"
    local source="${ARTIFACTS_SOURCE:-volume}"
    local s3_url="${ARTIFACTS_S3_URL:-}"
    local strip_components="${ARTIFACTS_STRIP_COMPONENTS:-1}"

    mkdir -p "${artifacts_dir}"

    dir_has_content() {
        find "${artifacts_dir}" -mindepth 1 -print -quit >/dev/null 2>&1
    }

    download_from_s3() {
        if [[ -z "${s3_url}" ]]; then
            echo "ARTIFACTS_SOURCE=s3 but ARTIFACTS_S3_URL is unset." >&2
            return 1
        fi

        if [[ "${artifacts_dir}" == "/" ]]; then
            echo "Refusing to clear root directory when preparing artifacts." >&2
            return 1
        fi

        rm -rf "${artifacts_dir}"
        mkdir -p "${artifacts_dir}"

        if [[ "${s3_url}" == *.tar.zst ]]; then
            curl -fSL --retry 3 --retry-delay 1 "${s3_url}" \
                | tar --extract --zstd --strip-components="${strip_components}" -C "${artifacts_dir}"
        elif [[ "${s3_url}" == *.tar.gz || "${s3_url}" == *.tgz ]]; then
            curl -fSL --retry 3 --retry-delay 1 "${s3_url}" \
                | tar --extract --gzip --strip-components="${strip_components}" -C "${artifacts_dir}"
        elif [[ "${s3_url}" == *.tar ]]; then
            curl -fSL --retry 3 --retry-delay 1 "${s3_url}" \
                | tar --extract --strip-components="${strip_components}" -C "${artifacts_dir}"
        else
            echo "Unsupported artifact archive format for ${s3_url}." >&2
            return 1
        fi
    }

    case "${source}" in
        volume)
            if ! dir_has_content; then
                echo "Expected pre-populated artifacts under ${artifacts_dir}. Provide a volume or set ARTIFACTS_SOURCE=s3." >&2
                return 1
            fi
            ;;
        s3)
            download_from_s3
            ;;
        none)
            :
            ;;
        *)
            echo "Unknown ARTIFACTS_SOURCE=${source}. Use volume|s3|none." >&2
            return 1
            ;;
    esac
}
