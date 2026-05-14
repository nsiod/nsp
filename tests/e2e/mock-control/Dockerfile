# Mock control center for the reverse-API e2e suite. Tiny Bun
# server that nsp's reverse-API client POSTs against, plus a
# `/__test__/*` interface the tester drives from the outside.
#
# Pinned to the same Bun release as the tester / UI workspace so
# image layers cache nicely on a single host.
FROM oven/bun:1.3.12-alpine

WORKDIR /mock
COPY server.ts ./

EXPOSE 9090
CMD ["bun", "run", "server.ts"]
