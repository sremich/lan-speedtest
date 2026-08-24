# TODO(milestone-0): replace this stub with the project's real build.
# Keep the VERSION/GIT_SHA build args — the release workflow passes them,
# and the app must surface them (web UI footer, --version, /api/status).

FROM alpine:3.20

ARG VERSION=dev
ARG GIT_SHA=unknown
ENV APP_VERSION=${VERSION} \
    APP_GIT_SHA=${GIT_SHA}

# TODO(milestone-0): build/copy the application here.

CMD ["sh", "-c", "echo \"scaffold stub — APP_VERSION=$APP_VERSION APP_GIT_SHA=$APP_GIT_SHA\""]
