# T2.9 Storyline posts: can one be verified without publishing?

Research from 2026-11-19. It is read-only: only the public web bundles were read, and nothing
was sent with authentication or posted.

## Verdict

**Possible through a draft, but it needs the user's go-ahead.** Nothing in either bundle offers
a preview, validate, dry-run or "only me" call for wall posts. The only unpublished state is
a draft, which is the same create mutation with `isDraft: true`. A draft is still a write on
the user's own storyline on the work account, so the sandbox-only rule rules it out unless
the user explicitly approves one throwaway draft and its deletion. Until the user approves,
T2.9 is **BLOCKED**.

**The current REST guess is unsafe.** `POST api.yammer.com/api/v1/messages.json {"body", "is_storyline_post": true}`
appears nowhere in either Engage bundle. On the legacy REST API, a message with no `group_id`
has gone to the All Company feed, so this body may publish company-wide instead of to the
storyline. `engage::storyline_body` / `Client::post_storyline` and `kelma-cli team storyline`
use it, so they must not be run. They should be replaced or disabled.

## Where the post is made

- **The Teams client posts nothing.**
  - "New storyline post" (`clickStorylineCreation` in the Teams `async-entry` chunk) only logs telemetry.
  - It then opens the app tab `com.microsoft.chattabs.storyline` in the self chat, with
    `subEntityId = type=custom,data=officeuserstorylinepublisher:<userObjectId>`.
  - Policy gates are `messagingPolicy.allowStorylinePost` and `teamsMessagingConfiguration.storyline`.
- **The tab is the Viva Engage web app** (`https://engage.cloud.microsoft/teams/storyline`).
  - It has public builds under `https://outlook-1.cdn.office.net/yammer/<build>/` and `.../yammer-teams-app/<build>/`.
  - `0-config.js` lists three hosts:
    - GraphQL: `https://engage.cloud.microsoft`
    - API: `https://api.engage.cloud.microsoft`
    - legacy: `https://www.yammer.com`
  - The token is `https://www.yammer.com/user_impersonation`.
- **The Engage client makes no REST post call.** Of the REST API it only uses `uploaded_files`, `translate`, `suggestions` and analytics.

## The mutation (best known, not run live)

- `POST https://engage.cloud.microsoft/graphql`, operation `PublishUserMessageClients`, field `createUserWallMessage`. The persisted-query sha256 is `1bd2f917…47bd1`.
- A post with no `groupId` from a viewer with `viewerCanPostToStoryline` is a storyline post. Community posts use `PublishGroupMessageClients` / `createGroupMessage`.
- Variables:
  - `userId`: the viewer's GraphQL id.
  - `serializedContentState`: a JSON string of a Draft.js raw state, `{"blocks":[{"text","type":"unstyled","depth":0,"inlineStyleRanges":[],"entityRanges":[],"data":{}}],"entityMap":{}}`.
  - `isDraft` and `clientMutationId`.
  - `isQuestion`, `isArticle` and `isUserMoment`, all `false`.
  - Include flags.
- Optional variables (leave out for a test): `draftReviewers`, `scheduledPublishAt`, `announcementAudience*`, `announcementNotificationTargets`, `title`, `topicNames`, `attachmentIds`, `onBehalfOfSenderId`.
- The result is `data.createUserWallMessage.message.thread` (`id`, `threadStarter.{id, viewerMutationId, isDraft}`).
- Related mutations:
  - Publish a draft: `editMessage(isDraft:false)` or `publishDraftThreads(threadIds)`. Never call these in a test.
  - Delete: `DeleteThreadClients` / `deleteThread(threadMutationId)` and `DeleteMessageClients` / `deleteMessage(messageMutationId)`.
- Read-only capability check: the `StorylinePublisherSettingsClients` query, plus `viewer.user.wallFeed.viewerCanStartThread` and `settings.storyline.viewerCanAccessStoryline`.

## Open points

- **Draft privacy.** That a draft is author-only (plus any reviewers added) is inferred from the client UI: the draft thread page, the pending-posts page and the reviewers list. It was not observed on the server.
- **Feature flag.** Save-as-draft is behind a client flag (probably the `SharedDrafts` flight), which was not resolved. If the flag is off for the tenant, the server might ignore `isDraft`, so the read-only capability query must come first.
- **Token audience.** kelma's `Audience::Yammer` uses resource `https://api.yammer.com`. Whether `engage.cloud.microsoft/graphql` accepts that token, or needs `https://www.yammer.com`, is unverified. A read-only GraphQL query settles it.
- **Transport.** The exact GraphQL transport (query string, headers, persisted-query handling) is unconfirmed, because the Relay environment chunk was not reachable.

## Safe test, only with the user's approval

1. Run a read-only GraphQL query (viewer id and storyline settings) to confirm the token and the capability.
2. Run `createUserWallMessage` with `isDraft: true` and no reviewers, schedule or announcement fields, and check that `threadStarter.isDraft == true`.
3. Delete the draft right away with `deleteThread`.
