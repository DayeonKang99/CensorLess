Node.js HTTP Proxy Agents Monorepo
==================================
[![Build Status](https://github.com/TooTallNate/proxy-agents/workflows/Node%20CI/badge.svg)](https://github.com/TooTallNate/proxy-agents/actions?workflow=Node+CI)

This monorepo contains various Node.js HTTP Agent implementations that operate over proxies using various protocols.

For the most common use-cases, you should be using the [`proxy-agent`](./packages/proxy-agent) module, which utilizes the other, more low-level, agent implementations.

You can find [changelogs here](CHANGELOG.md).


---
prerequisite: should have installed nodejs

1. curl -fsSL https://get.pnpm.io/install.sh | env PNPM_VERSION=10.0.0 sh -

2. cd to this repo

3. pnpm install

4. pnpm build

5. cd packages/proxy/dist/bin

6. node proxy -port 8008 (to check debug log: export NODE_ENV="debug"; node proxy.js --port 8080)

7. set proxy at settings
