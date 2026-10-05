# One-shot job: builds the module and publishes it to SpacetimeDB.
# Build context: backend/.
FROM node:22-slim
# The CLI must match the server version (see compose.yaml).
COPY --from=clockworklabs/spacetime:v2.10.1 /opt/spacetime /opt/spacetime
RUN ln -s /opt/spacetime/spacetimedb-cli /usr/local/bin/spacetime
WORKDIR /app
RUN chown 1000:1000 /app
COPY --chown=1000:1000 package.json package-lock.json* ./
# uid 1000 matches the server, so both share the spacetime-home volume.
USER 1000
RUN npm install --no-audit --no-fund
COPY --chown=1000:1000 . .
ENV HOME=/home/spacetime
CMD ["spacetime", "publish", "asimov", "--server", "http://spacetimedb:3000", "--module-path", "/app", "--no-config", "--yes"]
