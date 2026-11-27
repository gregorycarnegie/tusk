# To do

Day-to-day Net2 work moving into Tusk, so Net2 itself is left for advanced
configuration: doors and controllers, timezones, access level definitions,
public and custom days, operators and lockdown rules.

## Done

- [x] **Move people to another department.** Manage tab: tick people,
      choose a department. `PUT /users/{id}/departments`, the same request
      Add people already sends on a real Net2.
- [x] **Mark leavers.** Manage tab: optional move to a leavers department,
      then `GET /users/{id}` and `PUT /users/{id}` with only the expiry
      changed to their last day. Access that already ends sooner is
      never extended.
- [x] **Year change.** Manage tab: renames the year departments from the
      top down (`PUT /departments/{id}`), then makes the new lowest year
      (`POST /departments`). A new run requires one department per year in
      the selected range. Tests cover every stopping point, including ranges
      with mixed `Y7` and `Year 8` naming.

- [x] **Temporary access levels.** Manage tab, under Access level: give
      the ticked people the chosen level until a date and time
      (`POST /temporaryAccess/users/{id}` with
      `{userIds: [id], accessLevelId, expiryDateTime}`), check it
      (`GET /temporaryAccess/users/{id}`), and revoke it
      (`DELETE /temporaryAccess/users` with `[id]`), one person at a time.
      The department and revoke-everyone endpoints are left out: ticking a
      whole department does the same, and each row shows its own outcome.
- [x] **MFA: send the code from Tusk.** Net2's own Local API files show the
      flow (the DLL is obfuscated, so the names below come from its metadata):
      a password grant for an MFA operator throws `MfaRequiredException`, and
      the reply holds `error`, `userChallengeToken` and `userChallengeTypes`.
      Tusk takes a reply without `userChallengeToken` as no MFA, so those
      operators sign in with one request as before. With a challenge, Tusk
      sends `POST /authorization/mfa-request` `{challengeToken,
      challengeType}` (Email if offered, else the first type), asks for the
      code, checks it with `POST /authorization/mfa-challenge`
      `{challengeToken, code}`, then repeats the password grant with
      `challenge_token`. All three endpoints have their own 2 r/s nginx limit,
      so every call is form-urlencoded (no preflight).

## Check on a real Net2 before release

The browser smoke test exercises these workflows with a stand-in Net2.
Use a disposable Net2 database, a test person and test departments for the
release checks below.

Checked on a disposable Net2 database on 2026-11-25 (no doors configured):

- [x] `PUT /users/{id}` with the record from `GET /users/{id}` changes only
      the expiry: PIN, phone, custom fields, access levels, portrait,
      activation date, department and cards all stayed as they were.
- [x] `PUT /departments/{id}` with `{id, name}` renames the department, and
      its people stay in it.
- [x] `POST /departments` with `{name}` makes a department.
- [x] A full year change on test departments (Year 10 to 12), then run it
      again: nothing changed.
- [x] Undo a leaver: an explicit JSON `null` clears the expiry and keeps the
      rest of the record. Moving the expiry to a different date works.
- [x] Access levels: Net2 gives each person one access level. Sent
      several, it keeps the first and still answers 204. The Manage tab now
      sets or removes the one level and reads the set back.
- [x] Reviewed purge: the user and their cards are gone afterwards.
- [x] `sdk.UsersEx` has `LastKnownLocation`, not `LastArea`; the
      last-access query now uses it.

Still to check:

- [x] Expiry at a door (2026-11-26): Net2 stores the expiry as a date only
      (`T23:59:00` comes back as `T00:00:00`), but a card works on its
      expiry date and is refused the day after. Clearing the expiry restores
      access. The "23:59" wording is gone.
- [x] On a Net2 with doors (2026-11-26): site activity shows the last access
      and recent events, and an unknown card tapped at a door can be
      assigned and then opens it.
- [x] Door commands on a real door (2026-11-26): open once unlocks briefly,
      hold open keeps it unlocked, return to normal locks it again. Net2
      answers `FireAndForget` to all three, so it never confirms the outcome.
- [ ] Roll call reports and their entries (needs a roll call report in Net2).
- [x] Email MFA, checked with curl (2026-11-26). MFA is server-wide. The
      challenge is HTTP 403 `{error: "mfa_required", userChallengeTypes:
      ["Email"], userChallengeToken}`. `mfa-request` answers 202
      `{challengeType: 0, obfuscatedEmail, authenticatorAppSetupImage}` and
      sends the email; `mfa-challenge` answers 200 with an empty body. Net2
      decides with `IsMFARequired(operatorId)`: a passed code lets that
      operator's password through for the MFA "days" setting, on any device,
      and `challenge_token` makes no difference. With 0 days the API can
      never sign in (Tusk now says so). Both Net2 replies allow CORS.
- [x] MFA in Chrome through Tusk's sign-in sheet (2026-11-26): the code box
      named the masked address, the email came, and the code connected.
- [x] App MFA (2026-11-26): `mfa-request` with `App` answers a bare 400 until
      the operator has signed in to the Net2 desktop app once to scan the QR
      code (Tusk now says so), then 202 `{challengeType: 1}`; a fresh app
      code passes `mfa-challenge`. With Email turned off, Net2 still sends
      an email code if asked.
- [x] Renewing a session ignores MFA (2026-11-26): with the days at 0, the
      refresh grant still returned a new token without a challenge.
- [x] Temporary access, checked with curl (2026-11-26). GET answers
      `{userId, userHasTemporaryAccess}`. POST answers `{set: true}`, uses the
      path's user ID (body `userIds` is ignored), and *replaces* the person's
      access level; revoking (204) puts their own level back. When it runs
      out, Net2 puts their own level back by itself within two minutes.
- [x] Temporary access at a real door (2026-11-27, lift reader, level
      "Student Lift Hours"): the card opened while it was active, was refused
      a minute after it ended (09:54 yes, 09:56 no, end 09:55), and was
      refused straight after a revoke. Net2's own user screen shows what Tusk
      set as its "Temporary permissions".
- [x] Replace a card on the Manage tab with the desk reader (2026-11-25):
      the new card is saved and the old one is marked lost.
- [x] Add people with several `;`-separated access levels: Net2 kept only
      the first (2026-11-25), so Tusk now holds such rows back.

## Implemented and browser-tested

- [x] **Undo a leaver.** Change or clear the expiry for ticked people.
- [x] **Replace one person's card.** Tick them on Manage, tap the new card,
      and confirm. Old cards are marked lost after Net2 saves the new card.
- [x] **Set or remove the access level** of ticked people while preserving
      individual door permissions.
- [x] **Show a person's site activity** using `sdk.UsersEx` and `GET /events`.
- [x] **Show fire roll call reports** and their entries from Net2.
- [x] **Assign a card tapped at a door** using `events/latestunknowntokens`.
- [x] **Open, hold open, or return a door to normal operation** from Manage.
- [x] **Purge ticked leavers past the retention period** after downloading
      the review and confirming a second time. Recheck each user before
      sending `DELETE /users/{id}?purge=true`.
