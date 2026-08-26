import { describe, expect, it } from 'vitest';
import { MAX_LOCATION, matchExisting, mergeLocations, normaliseLocation } from './location';

describe('normaliseLocation', () => {
  it('trims, because the filter is an exact match and a stray space is invisible', () => {
    // "Office " and "Office" would be two rooms to the server and one room to
    // the person walking between them, with half the history in each.
    expect(normaliseLocation('  Office  ')).toBe('Office');
    expect(normaliseLocation('\tKitchen\n')).toBe('Kitchen');
  });

  it('collapses whitespace inside the tag as well as around it', () => {
    expect(normaliseLocation('Upstairs   landing')).toBe('Upstairs landing');
    expect(normaliseLocation('Back\tbedroom')).toBe('Back bedroom');
  });

  it('reports an empty tag for anything that is only whitespace', () => {
    // Which is what "no location" is: the run is stored untagged rather than
    // tagged with a blank.
    expect(normaliseLocation('')).toBe('');
    expect(normaliseLocation('   ')).toBe('');
    expect(normaliseLocation('\n\t ')).toBe('');
  });

  it('caps a tag at the length the backend will keep', () => {
    const long = 'a'.repeat(200);
    expect(normaliseLocation(long)).toHaveLength(MAX_LOCATION);
  });

  it('does not leave a trailing space when the cap lands on one', () => {
    // The cap is applied by cutting, which can cut mid-gap. A tag ending in a
    // space is exactly the thing the trim above exists to prevent, so the
    // truncation must not reintroduce it.
    const cut = `${'a'.repeat(MAX_LOCATION - 1)} bcdef`;
    expect(normaliseLocation(cut)).toBe('a'.repeat(MAX_LOCATION - 1));
  });

  it('keeps the case that was typed', () => {
    // The tag is a label a person reads, not a key they type. Lowercasing it
    // would silently rename their rooms.
    expect(normaliseLocation('Front Room')).toBe('Front Room');
  });
});

describe('matchExisting', () => {
  it('finds the spelling already in use, whatever case was typed', () => {
    const known = ['Office', 'Kitchen'];
    expect(matchExisting(known, 'office')).toBe('Office');
    expect(matchExisting(known, 'OFFICE')).toBe('Office');
    expect(matchExisting(known, 'Office')).toBe('Office');
  });

  it('reports nothing for a genuinely new room', () => {
    expect(matchExisting(['Office'], 'Garage')).toBeUndefined();
    expect(matchExisting([], 'Office')).toBeUndefined();
  });

  it('does not treat a different room as the same one', () => {
    // Substrings are different rooms: "Office" is not "Back office".
    expect(matchExisting(['Back office'], 'Office')).toBeUndefined();
  });
});

describe('mergeLocations', () => {
  it('offers a just-typed tag before the server has heard of it', () => {
    // The tag is chosen before the run that stores it, so at the moment it is
    // chosen the server's list cannot contain it. A row that dropped the
    // selected chip the instant it was selected would be unusable.
    expect(mergeLocations(['Office'], 'Garage')).toEqual(['Garage', 'Office']);
  });

  it('does not list a tag twice once the server knows it', () => {
    expect(mergeLocations(['Office', 'Kitchen'], 'Office')).toEqual(['Office', 'Kitchen']);
  });

  it('leaves the server order alone when nothing is selected', () => {
    // Most recently used first, which is the order the backend returns.
    expect(mergeLocations(['Office', 'Kitchen'], '')).toEqual(['Office', 'Kitchen']);
  });

  it('does not modify the list it was given', () => {
    const known = ['Office'];
    mergeLocations(known, 'Garage');
    expect(known).toEqual(['Office']);
  });
});
