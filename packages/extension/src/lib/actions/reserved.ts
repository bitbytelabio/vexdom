export const RESERVED_PREFIX = 'vexdom.';

export const START_RECORDING_ACTION = `${RESERVED_PREFIX}startRecording` as const;
export const STOP_RECORDING_ACTION = `${RESERVED_PREFIX}stopRecording` as const;
/** The agent hands the page to the user (a sign-in, a CAPTCHA) and waits until they say they are done. */
export const HAND_OFF_ACTION = `${RESERVED_PREFIX}handOff` as const;
/** In Follow-a-plan mode, the agent shows its sites and steps before acting. */
export const PROPOSE_PLAN_ACTION = `${RESERVED_PREFIX}proposePlan` as const;
