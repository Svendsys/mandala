---
name: do-code-review
description: >-
    Perform code review on a pull request. Read the corresponding GitHub issue if any to understand context. 
    Comment with your findings inline on the PR.
    Use this skill when asked to code-review, or review a PR.
---

# Do-code-review

You're being asked to perform a code review on a specific pull-request on GitHub. You should post all feedback directly
on the pull-request, not in the chat. 

### 1. Research the context
Check the repository conventions and philosophies, get a general understanding of what is being developed.
The PR likely (but not always) corresponds to a GitHub issue. If so, begin by reading the issue so that you understand
what is being asked for. Then read the PR description and any comments. Create a list of things that you want to assure
as you go through the code review.

### 2. Research the existing PR activity
If there are unclosed review comments there already then read them, and plan to (add to your list) verify them as you 
go through the review. If there are closed review comments, read them as well and plan to verify that they were actually 
solved, unless they were deemed irrelevant.

### 3. Do the code review
Review the code changes according to the research performed in #1 and #2. 
Are conventions and philosophies being respected?
Is the issue actually being solved as desired?
Does the code actually do what the PR author states it does?
Is the solution bloated and unnecessarily complex?
Has the author been "thinking out loud" in code comments anywhere?
Is the code adversarially tested or is it too naive / lazy?
Is the architecture and design of the project being respected?
Can you sincerely say that this is what the project maintainer likely had in mind?
And to the best of your abilities, evaluate the PR according to high level SWE standards.

### 4. Present your findings and conclusions
Post all your findings and comments inline wherever possible. Reply on any existing, open comments with your agreement
or disagreement and explain your reasoning. If you found any closed review comments that you think should not be closed,
reopen it and comment why you did so. Start each post or comment you make introducing yourself: 
"I am Claude <Model> <Version>". 

