# syntax=docker/dockerfile:1

# Start from Alpine Linux
FROM alpine:3.19

# Create an unprivileged user
ENV USER_ID=1000 USER_NAME=user USER_HOME=/home/user
RUN mkdir -v -p ${USER_HOME} && \
	addgroup -S -g ${USER_ID} ${USER_NAME} && \
	adduser -S -D -s /sbin/nologin -H -h ${USER_HOME} -g ${USER_NAME} -G ${USER_NAME} -u ${USER_ID} ${USER_NAME} && \
	chown -c -R ${USER_ID}:${USER_ID} ${USER_HOME}

# Add the build from context
ARG TARGETARCH
COPY --chown=0:0 --chmod=755 $TARGETARCH/darwin-push-port-live-feed /usr/local/bin/darwin-push-port-live-feed

# Configure reasonable defaults
ENV DARWIN_PORT=61613

# Launch the build
ENTRYPOINT [ "/usr/local/bin/darwin-push-port-live-feed" ]
