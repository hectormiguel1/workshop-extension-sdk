Feature: Packed storage records
  `#[derive(PackedRecord)]` reads and writes the packed format byte for byte:
  the golden records below were written by a released warp unit's own
  hand-written encoder.

  Scenario: The warp unit's options derive to the frozen layout
    Then the derived options layout equals the warp unit's schema 2 fields

  Scenario: Golden records decode and re-encode identically
    Then the default options encode to "0101000000be13000000000000000000"
    And the full options round-trip "012bc031c0d229e9ffffff7f0d000020000c080052657475726e4d61705365747a3130355f61726b00090a0052657475726e506f73312e35202d3220333030030504004f746865726b657074"
    And the schema 1 record "010a0000007c27803401000000000000" migrates to destination 5 and return zone 1234
    And an unknown schema is refused

  Scenario Outline: Malformed records are refused
    Then the options record "<hex>" is refused for <defect>

    Examples:
      | defect                  | hex                                                      |
      | another version         | 0201000000be13000000000000000000                         |
      | too short               | 0101000000be13                                           |
      | destination 15 above 11 | 010f000000be13000000000000000000                         |
      | a padding bit           | 0101000000be13000000000000000080                         |
      | a truncated text header | 0101000000be130000000000000000000001                     |
      | a key past the end      | 0101000000be13000000000000000000000300006162             |
      | a value past the end    | 0101000000be13000000000000000000000105006162             |
      | section 5               | 0101000000be130000000000000000000501000061               |
      | an empty key            | 0101000000be1300000000000000000000000000                 |
      | a 4097-byte value       | 0101000000be130000000000000000000001011061               |
      | a key naming a field    | 0101000000be1300000000000000000000080000426174746c656964 |
      | a duplicate key         | 0101000000be1300000000000000000000010000610001000041     |
      | a NUL in a value        | 0101000000be13000000000000000000000101006100             |
      | a non-UTF-8 key         | 0101000000be1300000000000000000000010000ff               |
      | a non-UTF-8 value       | 0101000000be130000000000000000000001010061ff             |

  Scenario: Encoding refuses what decoding would refuse
    Then encoding refuses short values, out-of-range values, bad or duplicate text, oversized and overlong records
    And a record failing its validity hook neither encodes nor decodes
    And an oversized record and too many text entries are refused on decode

  Scenario: Records without text or hooks use the trait defaults
    Then a minimal record packs to 2 bytes and back
    And invalid layouts are detected
    And every integer type converts
