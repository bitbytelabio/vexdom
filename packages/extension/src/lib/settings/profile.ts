import { jsonText, profileAsUser, profileIsEmpty, profileIsUser, profileNormalize } from '@/lib/bridge/pure';

export interface ProfileField {
  readonly id: string;
  readonly label: string;
  readonly autocomplete: string;
  readonly type?: 'email' | 'tel';
  readonly multiline?: boolean;
}

export const PROFILE_FIELDS = [
  { id: 'givenName', label: 'First name', autocomplete: 'given-name' },
  { id: 'familyName', label: 'Last name', autocomplete: 'family-name' },
  { id: 'email', label: 'Email', autocomplete: 'email', type: 'email' },
  { id: 'phone', label: 'Phone', autocomplete: 'tel', type: 'tel' },
  { id: 'street', label: 'Street address', autocomplete: 'street-address', multiline: true },
  { id: 'city', label: 'City', autocomplete: 'address-level2' },
  { id: 'region', label: 'State / region', autocomplete: 'address-level1' },
  { id: 'postalCode', label: 'Postal code', autocomplete: 'postal-code' },
  { id: 'country', label: 'Country', autocomplete: 'country-name' },
  { id: 'company', label: 'Company', autocomplete: 'organization' },
  { id: 'jobTitle', label: 'Job title', autocomplete: 'organization-title' },
] as const satisfies readonly ProfileField[];

export type ProfileFieldId = (typeof PROFILE_FIELDS)[number]['id'];

export interface ProfileDetail {
  readonly label: string;
  readonly value: string;
}

export interface UserProfile {
  readonly fields: Partial<Record<ProfileFieldId, string>>;
  readonly details: readonly ProfileDetail[];
  readonly instructions: string;
}

export const PROFILE_LIMITS = { value: 300, label: 60, details: 20, instructions: 4000 } as const;

export const EMPTY_PROFILE: UserProfile = { fields: {}, details: [], instructions: '' };

const FIELD_IDS: readonly string[] = PROFILE_FIELDS.map((field) => field.id);

export function isProfileFieldId(value: string): value is ProfileFieldId {
  return FIELD_IDS.includes(value);
}

/**
 * The checks and the normal form come from `vexdom-pure` (through ext-core), the same code the
 * Bridge reads `profile.json` with. Synchronous, so ext-core must be loaded (`loadCore()`) first.
 */
export function isUserProfile(value: unknown): value is UserProfile {
  return profileIsUser(jsonText(value));
}

export function normalizeProfile(profile: UserProfile): UserProfile {
  return JSON.parse(profileNormalize(jsonText(profile))) as UserProfile;
}

/** Drops what does not fit rather than cutting it short: a truncated phone number is worse than none. */
export function asUserProfile(value: unknown): UserProfile {
  return JSON.parse(profileAsUser(jsonText(value))) as UserProfile;
}

export function isEmptyProfile(profile: UserProfile): boolean {
  return profileIsEmpty(jsonText(profile));
}

export function sameProfile(a: UserProfile, b: UserProfile): boolean {
  return JSON.stringify(normalizeProfile(a)) === JSON.stringify(normalizeProfile(b));
}
