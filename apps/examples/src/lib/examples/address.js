const addressRules = `rule "CustomerMatchedFromReferenceData" salience 80 no-loop {
    when
        c_ref_match_score(AddressDecision.source_text, "customer") >= 450
    then
        AddressDecision.reference_status = "matched";
}

rule "CustomerNotFoundInReferenceData" salience 70 no-loop {
    when
        c_ref_match_count(AddressDecision.source_text, "customer") <= 0
    then
        AddressDecision.reference_status = "not_found";
        AddressDecision.reference_name = "";
}

rule "ProductMatchedFromReferenceData" salience 60 no-loop {
    when
        c_ref_lexical_score(AddressDecision.source_text, "product") >= 450
    then
        Decision.product_reference_matched = true;
}

rule "AddressFoundInLocalIndex" salience 20 no-loop {
    when
        c_addr_index_score(AddressDecision.standardized) >= 850 &&
        AddressDecision.policy_status == "pending"
    then
        AddressDecision.policy_status = "valid_by_address_index";
        AddressDecision.policy_reason = "Address matched the local OpenAddresses-style index.";
}

rule "KingColaBrandOwnerBillToInvalid" salience 100 no-loop {
    when
        m_ref_match_name(AddressDecision.source_text, "customer") == "King Cola" &&
        AddressDecision.role == "bill_to"
    then
        AddressDecision.policy_status = "invalid";
        AddressDecision.policy_reason = "King Cola is the brand owner, so bill-to must be a distributor account selected by context.";
}

rule "TristateColaKnownBillToException" salience 90 no-loop {
    when
        AddressDecision.customer == "Tristate Cola" &&
        AddressDecision.role == "bill_to" &&
        AddressDecision.standardized contains "111 East Cola Lane"
    then
        AddressDecision.policy_status = "accepted_by_policy";
        AddressDecision.policy_reason = "Tristate Cola uses 111 East Cola Lane as its bill-to address even when postal confidence is not perfect.";
}

rule "PostalProfileInvalid" salience 10 no-loop {
    when
        AddressDecision.address_valid == false &&
        b_addr_index_match(AddressDecision.standardized) == false
    then
        AddressDecision.policy_status = "invalid";
        AddressDecision.policy_reason = "Address content does not satisfy the minimum postal profile.";
}`;

export const addressVerificationExample = {
  text: 'Please validate this 12 pack box order for King Cola. The requested bill-to is 500 Royal Road, Springfield IL 62701.',
  structured: {
    customer_name: 'Tristate Cola',
    purpose: 'bill to',
    'addr gobblygook': '111 East Cola Lane',
    municipality: 'Springfield',
    state_province: 'IL',
    postalish: '62701',
    country: 'US'
  },
  rules: addressRules,
  addressIndex: [
    {
      id: 'oa-demo:tristate-billto',
      source: {
        NUMBER: '111',
        STREET: 'East Cola Lane',
        CITY: 'Springfield',
        REGION: 'IL',
        POSTCODE: '62701',
        source_license: 'demo fixture'
      }
    },
    {
      id: 'oa-demo:king-royal-road',
      source: {
        NUMBER: '500',
        STREET: 'Royal Road',
        CITY: 'Springfield',
        REGION: 'IL',
        POSTCODE: '62701',
        source_license: 'demo fixture'
      }
    }
  ],
  referenceIndex: [
    {
      id: 'customer:king-cola',
      kind: 'customer',
      name: 'King Cola',
      aliases: ['King Cola', 'King Cola Company'],
      customer_type: 'brand_owner',
      policy_note: 'Brand owner; bill-to is invalid because distributors are billed.'
    },
    {
      id: 'customer:queen-cola',
      kind: 'customer',
      name: 'Queen Cola',
      aliases: ['Queen Cola', 'Queen Cola Company'],
      customer_type: 'brand_owner',
      policy_note: 'Reference match only; no King Cola policy applies.'
    },
    {
      id: 'customer:tristate-cola',
      kind: 'customer',
      name: 'Tristate Cola',
      aliases: ['Tristate Cola'],
      customer_type: 'distributor',
      policy_note: 'Distributor with known bill-to exception.'
    },
    {
      id: 'product:6-pack-box',
      kind: 'product',
      name: '6 pack box',
      aliases: ['6 pack box', 'six pack box', '6-pack box'],
      packaging: 'box'
    },
    {
      id: 'product:12-pack-box',
      kind: 'product',
      name: '12 pack box',
      aliases: ['12 pack box', 'twelve pack box', '12-pack box'],
      packaging: 'box'
    },
    {
      id: 'product:case-shrink-wrap',
      kind: 'product',
      name: 'case shrink wrap',
      aliases: ['case shrink wrap', 'shrink wrapped case', 'case wrap'],
      packaging: 'shrink_wrap'
    }
  ]
};
