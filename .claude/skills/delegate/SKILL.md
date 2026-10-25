---
name: delegate
description: >-
    Delegate an issue to a sub-agent (worker). You should select an appropriate model for the worker, and follow up to
    ensure that the worker is correctly understanding the issue. The worker should run /implement on the issue.
    Use this skill whenever you're delegating work.
---

# Delegate

You are delegating work to a sub-agent and must ensure that a strong enough model is being used for the task.
You are also responsible to follow up the work the sub-agent is doing and ensure that it's being done right. 
When the worker has created a PR, you should also launch an independent code-review agent to review the work 
using /do-code-review. And after the review has been done, ensure that the sub-agent in charge of the issue is 
following up on the review.

### 1. Research the context
Look up the issue being delegated, and its parent issue if any. Read all the info including comments so that you have
a good understanding.

### 2. Decide on the right model
The issues may already define a recommended model, but no one has a better overview than you at this point.
Do not use Haiku unless it's purely for quick research and summary tasks. For Sonnet always use High effort or better.
For Opus medium effort is acceptable for some tasks. Generally speaking, our list goes like:
Sonnet (high effort), Opus (medium effort), Opus (high effort), Opus (extra effort), Fable (high effort). Fable may not
always be available, if it isn't then use Opus (extra effort) instead. For tasks that involve important, fundamental
parts that much will later build upon or rely upon, go with Opus or higher.

### 3. Delegate the issue
When an appropriate model has been selected, spawn the agent and instruct it to run /implement on the issue. You may 
also have additional instructions for the model that you can give, if for example you feel that the model should be
aware of something that the issue does not mention.

### 4. Follow up
The worker agent may have questions for you, or may have other problems or misunderstandings. It is your job to follow
up and ensure that everything is going according to plan. When the worker agent is ready to create a PR, see to it that
it actually does so.

### 5. Have the PR code-reviewed
When a PR is up, spawn an independent worker agent to review it using /do-code-review. Make sure that you use a strong
model to do the PR. For example if the PR author is Sonnet, use Opus to review it. After the review is done, the PR 
author should automatically begin addressing the review. If it doesn't then make sure it does. When the author has
addressed all findings, if it's a high stakes issue, run another review round. Whether to run additional review rounds
is left to your discretion, depending on the severity, size, and confidence in the PR.

### 6. Merge the PR
When the PR has been thoroughly reviewed and quality-assured, merge it. Then mark the issue as done, and retire all
sub agents who were involved. If the PR has merge conflicts, personally resolve the conflicts then merge.
