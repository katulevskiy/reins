Reins
Reins is on
Dana's iPhone approves
Home
⌘1
Activity
⌘2
Connections
⌘3
Keys
⌘4
Rules
⌘5
Settings
⌘6
Pause git
Keys

Secrets live on your phone. Only their names live here.

Open config.toml
API KEYS
openai
vault:OpenAI/api key · held 1 h
Held until 22:34
anthropic
vault:Anthropic/api key · held 30 min
Asks next time
RUN PROFILES
deploy
Deploy the web app to production · 2 secrets
AWS_SECRET_ACCESS_KEY
←
vault:AWS deploy/password
SENTRY_AUTH_TOKEN
←
vault:Sentry/auth token
release
Publish packages · 1 secret
SSH AGENT
SSH keys from your phone
~/.reins/ssh-agent.sock
MacBook key
SHA256:x1Pq7nE…9fZk
Deploy key
SHA256:Hc04LmW…a2Qe
USE THEM
# An API, its key from your phone
OPENAI_BASE_URL=http://127.0.0.1:7457/api/openai
# A program with secrets
reins run --profile deploy -- ./deploy.sh
# ssh with your phone's keys
export SSH_AUTH_SOCK=~/.reins/ssh-agent.sock