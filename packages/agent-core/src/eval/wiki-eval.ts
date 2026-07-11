export interface WikiEvalCase {
  id: string;
  prompt: string;
  checks: WikiEvalChecks;
}

export interface WikiEvalChecks {
  requiredToolCalls?: string[];
  forbiddenToolCalls?: string[];
  requiredRecipeResultIds?: string[];
  forbiddenRecipeCards?: boolean;
  requiredText?: string[];
  requiredAnyText?: string[][];
  forbiddenVisiblePatterns?: string[];
}

export interface WikiEvalToolCall {
  name: string;
  input?: unknown;
  output?: unknown;
}

export interface WikiEvalTranscript {
  finalText: string;
  toolCalls: WikiEvalToolCall[];
  errors?: string[];
}

export interface WikiEvalCheckResult {
  name: string;
  passed: boolean;
  detail: string;
}

export interface WikiEvalVerdict {
  caseId: string;
  passed: boolean;
  checks: WikiEvalCheckResult[];
}

export function extractRecipeCards(text: string): any[] {
  const cards: any[] = [];
  const re = /```recipe_card\s*([\s\S]*?)```/g;
  for (const match of text.matchAll(re)) {
    try {
      cards.push(JSON.parse(match[1] ?? ""));
    } catch {
      // Invalid cards are handled by required-card checks failing.
    }
  }
  return cards;
}

export function evaluateWikiEvalTranscript(
  testCase: WikiEvalCase,
  transcript: WikiEvalTranscript,
): WikiEvalVerdict {
  const checks: WikiEvalCheckResult[] = [];
  const toolNames = transcript.toolCalls.map((call) => call.name);
  const recipeCards = extractRecipeCards(transcript.finalText);
  const visibleText = stripRecipeCardBlocks(transcript.finalText);

  pushCheck(
    checks,
    "no_runtime_errors",
    !transcript.errors?.length,
    transcript.errors?.join("; ") || "no errors",
  );

  for (const name of testCase.checks.requiredToolCalls ?? []) {
    pushCheck(
      checks,
      `called_${name}`,
      toolNames.includes(name),
      toolNames.includes(name) ? "observed" : `observed: ${toolNames.join(", ") || "none"}`,
    );
  }

  for (const name of testCase.checks.forbiddenToolCalls ?? []) {
    pushCheck(
      checks,
      `did_not_call_${name}`,
      !toolNames.includes(name),
      toolNames.includes(name) ? "unexpected call observed" : "not observed",
    );
  }

  for (const text of testCase.checks.requiredText ?? []) {
    pushCheck(
      checks,
      `contains_${text}`,
      containsFolded(transcript.finalText, text),
      containsFolded(transcript.finalText, text) ? "present" : "missing",
    );
  }

  for (const alternatives of testCase.checks.requiredAnyText ?? []) {
    const found = alternatives.some((text) => containsFolded(transcript.finalText, text));
    pushCheck(
      checks,
      `contains_any_${alternatives.join("_or_")}`,
      found,
      found ? "present" : `missing all of: ${alternatives.join(", ")}`,
    );
  }

  for (const pattern of testCase.checks.forbiddenVisiblePatterns ?? []) {
    pushCheck(
      checks,
      `visible_text_omits_${pattern}`,
      !visibleText.includes(pattern),
      visibleText.includes(pattern) ? "visible leak observed" : "not visible",
    );
  }

  if (testCase.checks.forbiddenRecipeCards) {
    pushCheck(
      checks,
      "no_recipe_card",
      recipeCards.length === 0,
      recipeCards.length === 0 ? "none" : `${recipeCards.length} card(s) observed`,
    );
  }

  for (const id of testCase.checks.requiredRecipeResultIds ?? []) {
    const found = recipeCards.some((card) => card?.result?.id === id);
    pushCheck(
      checks,
      `recipe_result_${id}`,
      found,
      found
        ? "present"
        : `observed: ${recipeCards.map((card) => card?.result?.id ?? "?").join(", ") || "none"}`,
    );
  }

  return {
    caseId: testCase.id,
    passed: checks.every((check) => check.passed),
    checks,
  };
}

function pushCheck(
  checks: WikiEvalCheckResult[],
  name: string,
  passed: boolean,
  detail: string,
): void {
  checks.push({ name, passed, detail });
}

function stripRecipeCardBlocks(text: string): string {
  return text.replace(/```recipe_card\s*[\s\S]*?```/g, "");
}

function containsFolded(haystack: string, needle: string): boolean {
  return haystack.toLocaleLowerCase().includes(needle.toLocaleLowerCase());
}
