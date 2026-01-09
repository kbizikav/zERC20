// SPDX-License-Identifier: GPL-3.0
pragma solidity >=0.7.0 <0.9.0;

/*
    Sonobe's Nova + CycleFold decider verifier.
    Joint effort by 0xPARC & PSE.

    More details at https://github.com/privacy-scaling-explorations/sonobe
    Usage and design documentation at https://privacy-scaling-explorations.github.io/sonobe-docs/

    Uses the https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs
    Groth16 verifier implementation and a KZG10 Solidity template adapted from
    https://github.com/weijiekoh/libkzg.
    Additionally we implement the RootNovaDecider contract, which combines the
    Groth16 and KZG10 verifiers to verify the zkSNARK proofs coming from
    Nova+CycleFold folding.
*/

/* =============================== */
/* KZG10 verifier methods */
/**
 * @author  Privacy and Scaling Explorations team - pse.dev
 * @dev     Contains utility functions for ops in BN254; in G_1 mostly.
 * @notice  Forked from https://github.com/weijiekoh/libkzg.
 * Among others, a few of the changes we did on this fork were:
 * - Templating the pragma version
 * - Removing type wrappers and use uints instead
 * - Performing changes on arg types
 * - Update some of the `require` statements
 * - Use the bn254 scalar field instead of checking for overflow on the babyjub prime
 * - In batch checking, we compute auxiliary polynomials and their commitments at the same time.
 */
contract KZG10Verifier {
    // prime of field F_p over which y^2 = x^3 + 3 is defined
    uint256 public constant BN254_PRIME_FIELD =
        21888242871839275222246405745257275088696311157297823662689037894645226208583;
    uint256 public constant BN254_SCALAR_FIELD =
        21888242871839275222246405745257275088548364400416034343698204186575808495617;

    /**
     * @notice  Performs scalar multiplication in G_1.
     * @param   p  G_1 point to multiply
     * @param   s  Scalar to multiply by
     * @return  r  G_1 point p multiplied by scalar s
     */
    function mulScalar(uint256[2] memory p, uint256 s) internal view returns (uint256[2] memory r) {
        uint256[3] memory input;
        input[0] = p[0];
        input[1] = p[1];
        input[2] = s;
        bool success;
        assembly {
            success := staticcall(sub(gas(), 2000), 7, input, 0x60, r, 0x40)
            switch success
            case 0 { invalid() }
        }
        require(success, "bn254: scalar mul failed");
    }

    /**
     * @notice  Negates a point in G_1.
     * @param   p  G_1 point to negate
     * @return  uint256[2]  G_1 point -p
     */
    function negate(uint256[2] memory p) internal pure returns (uint256[2] memory) {
        if (p[0] == 0 && p[1] == 0) {
            return p;
        }
        return [p[0], BN254_PRIME_FIELD - (p[1] % BN254_PRIME_FIELD)];
    }

    /**
     * @notice  Adds two points in G_1.
     * @param   p1  G_1 point 1
     * @param   p2  G_1 point 2
     * @return  r  G_1 point p1 + p2
     */
    function add(uint256[2] memory p1, uint256[2] memory p2) internal view returns (uint256[2] memory r) {
        bool success;
        uint256[4] memory input = [p1[0], p1[1], p2[0], p2[1]];
        assembly {
            success := staticcall(sub(gas(), 2000), 6, input, 0x80, r, 0x40)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: point add failed");
    }

    /**
     * @notice  Computes the pairing check e(p1, p2) * e(p3, p4) == 1
     * @dev     Note that G_2 points a*i + b are encoded as two elements of F_p, (a, b)
     * @param   a_1  G_1 point 1
     * @param   a_2  G_2 point 1
     * @param   b_1  G_1 point 2
     * @param   b_2  G_2 point 2
     * @return  result  true if pairing check is successful
     */
    function pairing(uint256[2] memory a_1, uint256[2][2] memory a_2, uint256[2] memory b_1, uint256[2][2] memory b_2)
        internal
        view
        returns (bool result)
    {
        uint256[12] memory input = [
            a_1[0],
            a_1[1],
            a_2[0][1], // imaginary part first
            a_2[0][0],
            a_2[1][1], // imaginary part first
            a_2[1][0],
            b_1[0],
            b_1[1],
            b_2[0][1], // imaginary part first
            b_2[0][0],
            b_2[1][1], // imaginary part first
            b_2[1][0]
        ];

        uint256[1] memory out;
        bool success;

        assembly {
            success := staticcall(sub(gas(), 2000), 8, input, 0x180, out, 0x20)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: pairing failed");

        return out[0] == 1;
    }

    uint256[2] G_1 = [
        17972166172477927580966753801534392926390414011182124296494436899875835459482,
        2218271212538119924126019821815265793826772103153839241786596125668022500874
    ];
    uint256[2][2] G_2 = [
        [
            1154064315684751160495224059341546990274586129569136226981447946927474010357,
            11377576883007562778368087203556526979100510756599572273371116245959322242842
        ],
        [
            14072542809811940477759791225745094028924715801976337561770589948910340746824,
            10297242815337090708932145440564825178130493348230954535537624862979123603686
        ]
    ];
    uint256[2][2] VK = [
        [
            8603883509550290673906136255186266043975262925158834546291170106521717107984,
            19475934799171192024959593067512424885589043002681463988190298614439941798898
        ],
        [
            663358650747201254546605401784794689975749059669526883354234761642933361929,
            6404900493521595653073306930410958023699398323275000203997306403253929532381
        ]
    ];

    /**
     * @notice  Verifies a single point evaluation proof. Function name follows `ark-poly`.
     * @dev     To avoid ops in G_2, we slightly tweak how the verification is done.
     * @param   c  G_1 point commitment to polynomial.
     * @param   pi G_1 point proof.
     * @param   x  Value to prove evaluation of polynomial at.
     * @param   y  Evaluation poly(x).
     * @return  result Indicates if KZG proof is correct.
     */
    function check(uint256[2] calldata c, uint256[2] calldata pi, uint256 x, uint256 y)
        public
        view
        returns (bool result)
    {
        //
        // we want to:
        //      1. avoid gas intensive ops in G2
        //      2. format the pairing check in line with what the evm opcode expects.
        //
        // we can do this by tweaking the KZG check to be:
        //
        //          e(pi, vk - x * g2) = e(c - y * g1, g2) [initial check]
        //          e(pi, vk - x * g2) * e(c - y * g1, g2)^{-1} = 1
        //          e(pi, vk - x * g2) * e(-c + y * g1, g2) = 1 [bilinearity of pairing for all subsequent steps]
        //          e(pi, vk) * e(pi, -x * g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(-x * pi, g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(x * -pi - c + y * g1, g2) = 1 [done]
        //                        |_   rhs_pairing  _|
        //
        uint256[2] memory rhs_pairing = add(mulScalar(negate(pi), x), add(negate(c), mulScalar(G_1, y)));
        return pairing(pi, VK, rhs_pairing, G_2);
    }

    function evalPolyAt(uint256[] memory _coefficients, uint256 _index) public pure returns (uint256) {
        uint256 m = BN254_SCALAR_FIELD;
        uint256 result = 0;
        uint256 powerOfX = 1;

        for (uint256 i = 0; i < _coefficients.length; i++) {
            uint256 coeff = _coefficients[i];
            assembly {
                result := addmod(result, mulmod(powerOfX, coeff, m), m)
                powerOfX := mulmod(powerOfX, _index, m)
            }
        }
        return result;
    }
}

/* =============================== */
/* Groth16 verifier methods */
/*
    Copyright 2021 0KIMS association.

    * `solidity-verifiers` added comment
        This file is a template built out of [snarkJS](https://github.com/iden3/snarkjs) groth16 verifier.
        See the original ejs template [here](https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs)
    *

    snarkJS is a free software: you can redistribute it and/or modify it
    under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    snarkJS is distributed in the hope that it will be useful, but WITHOUT
    ANY WARRANTY; without even the implied warranty of MERCHANTABILITY
    or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public
    License for more details.

    You should have received a copy of the GNU General Public License
    along with snarkJS. If not, see <https://www.gnu.org/licenses/>.
*/

contract Groth16Verifier {
    // Scalar field size
    uint256 constant r = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    // Base field size
    uint256 constant q = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // Verification Key data
    uint256 constant alphax = 14063568825121222987252665438080124095479660142462128354500855392369927740947;
    uint256 constant alphay = 1036641240717259284197444372481248193261754753028574343243185520684149044826;
    uint256 constant betax1 = 12259590911409424800440662053117587818364164875239061271667150572566487099819;
    uint256 constant betax2 = 8478195762251265088060257755499907823997000555748059046429501634749341767618;
    uint256 constant betay1 = 20283521014826873422244158172279299338628770163953232805445030358540471324375;
    uint256 constant betay2 = 10746896537489857311282020155533382705492447691112948305864240101443447648890;
    uint256 constant gammax1 = 18649285157148027970461862994407973604219964481163434659244824888267758488017;
    uint256 constant gammax2 = 1575722106320445668811215411384293063364624325724406753258425680987761134159;
    uint256 constant gammay1 = 14129827191832349177583864971360710990455293903558908627000487882155733162758;
    uint256 constant gammay2 = 10688633216469393180525166974913388592252465644911013330400707150946468636577;
    uint256 constant deltax1 = 10538651478149458989849339986313735088474632270671711023070564110943669803218;
    uint256 constant deltax2 = 1305467539439163997533585740862968572346371551463775903177786773622991246826;
    uint256 constant deltay1 = 14752496234907181405815200002350513382346626323944746000857471514379525076102;
    uint256 constant deltay2 = 15564852097701493536090688030437847912072338853093368342919193095581144941270;

    uint256 constant IC0x = 9029110650201148835788697630922062489105180178820885979180763776779339168307;
    uint256 constant IC0y = 20589398844799511897729963613622142210727803696557939866376128949474513424834;

    uint256 constant IC1x = 7923820371759030227859696667610437192908063782271298692930960646253092850403;
    uint256 constant IC1y = 3136388856239895168161849317263630832239177634128560034124888576024630492180;

    uint256 constant IC2x = 1675071889344156253302765882121185817360682450606104216401133394986161598692;
    uint256 constant IC2y = 1113591600439556878352714739724902691615744470521688601731091369288508978399;

    uint256 constant IC3x = 20419685659516152556653260084989931025203387114291094598934610114871163967110;
    uint256 constant IC3y = 8553974154081136827486093932716164544077591754408144406113039698383749850277;

    uint256 constant IC4x = 15842792972944543564867304068180662326260694028412678500660227236378077570023;
    uint256 constant IC4y = 13130708083136671627620393146499089006052428173755721365821750782795428480634;

    uint256 constant IC5x = 10847655673618017529586476714069152807975719936969421610649027017100725533715;
    uint256 constant IC5y = 1418732245598013964601100985250771615585351984294649956549871686455002077182;

    uint256 constant IC6x = 21600226557299831314826754965114561011372827830207233096822780163940310887284;
    uint256 constant IC6y = 6844935785930819343742747845858228780295241610478311623035055410741619423725;

    uint256 constant IC7x = 16800705406244435590416830982350429034394415778619907763152634086702825086360;
    uint256 constant IC7y = 9325129392826156904526946130144021651399135506835243526441231379869751059588;

    uint256 constant IC8x = 3073150233803694133141676845101966623551709752024396949898810597547341442765;
    uint256 constant IC8y = 19504540884800544010686046844806988622762509010627863754670560670164602726866;

    uint256 constant IC9x = 20359570396392781123196799694117728473251696755580158371844692310029149586328;
    uint256 constant IC9y = 18316233856514835190419489462559106319866969929340092251135777204575583112157;

    uint256 constant IC10x = 13487164713889044119972018840435769009164700563323581243206740997694172579706;
    uint256 constant IC10y = 18381029085505369216506627795257809238875824025214855992932186763622506160726;

    uint256 constant IC11x = 11014029995913652919613876245183233272855199640571872948259317374247866675686;
    uint256 constant IC11y = 14833017323734404324162269055075494589239089208369746476396618413483731879056;

    uint256 constant IC12x = 7290439592806800842566889256652298930902158642860245164514383870560601448404;
    uint256 constant IC12y = 2671090606303380514789568172856276529816595410788659572373653371651847809514;

    uint256 constant IC13x = 5713348764281501770642664645919210147626262249670365585162406604188074514124;
    uint256 constant IC13y = 18642497082665649805546461356314785735605471771100043737626378769458307848041;

    uint256 constant IC14x = 418095376424158202167179910917688787676409460921146661881360650835463232920;
    uint256 constant IC14y = 3474109633700078970641593519701796743119828393785088482092809153551769943979;

    uint256 constant IC15x = 17072793956673260894921087966840798599259855918802669037964065460578233131998;
    uint256 constant IC15y = 17069904442881816690634314850917966354528265920361940966810841169057742871393;

    uint256 constant IC16x = 19680573048206391930663703835567799172126545963440817372599892236520898930406;
    uint256 constant IC16y = 9398603924646182437593317397703753313860206879902237454531917684961658346463;

    uint256 constant IC17x = 6991921022053314683315728250929437379794968998178355699819379507301743477773;
    uint256 constant IC17y = 14487110819004399515612823730340793737136418888661580583888070119466886145617;

    uint256 constant IC18x = 6201505546394280551294816489845861079478273330830791834923882265265801618206;
    uint256 constant IC18y = 21842277336621605686086762774275651949368728118386024307125794420278668509938;

    uint256 constant IC19x = 8291506271576612856054667210537121470467516754930276166374352117514364071004;
    uint256 constant IC19y = 2236302934254585015369986852256490916869581839737120323481110650006188994658;

    uint256 constant IC20x = 15273837585768164308433345808624803014769923474475460836180111500602116029881;
    uint256 constant IC20y = 5179995793472528677629869767724404499953773102844795777545526693213552546549;

    uint256 constant IC21x = 5015333065844908051517049341095618910021454351927699130653650956852833858033;
    uint256 constant IC21y = 1726007902023057298656421130402423461586569193533330140148299891358781538333;

    uint256 constant IC22x = 19999058557064615120112198698876993381578191713571392394878157050350135680803;
    uint256 constant IC22y = 19689355643456087764729625262076602598254725578325552370581676717359568911179;

    uint256 constant IC23x = 2691053017269995578270176582689434550546357283914991283812972200999553678823;
    uint256 constant IC23y = 6596619010943678814935996624333978390959170157284379183456340478525709693701;

    uint256 constant IC24x = 10926358629013877586788405752595870670601755344468076407679304212374071265438;
    uint256 constant IC24y = 6474106785233149383732246340816826937416116036484954161267380202924914003090;

    uint256 constant IC25x = 16856460677893984946578090752229432882558784522974972728512662517360621873188;
    uint256 constant IC25y = 4853242394155929995196886073684096290758538209419586733641760065753923287844;

    uint256 constant IC26x = 12387220366876747124662286415457993751718292952507478485100944321989165521224;
    uint256 constant IC26y = 14614532970503306530899845263761798199164682994314446543854923202194933788885;

    uint256 constant IC27x = 12041117230379857676521627878689115563356138957946622516723694000393949553178;
    uint256 constant IC27y = 18174835129507388022052662725676257805847676356774315045955500643158993041229;

    uint256 constant IC28x = 13310763512292937127508247021835127897768924162515018210686971876917555867141;
    uint256 constant IC28y = 16205178791269768050642986014196345006634954201439324198200238165090716377206;

    uint256 constant IC29x = 20486673136544987501032457641618627277591967852063360143959205873436556934263;
    uint256 constant IC29y = 9144109892597855380575379130434757472990498132916191927830251024013584798565;

    uint256 constant IC30x = 15805294100089700936105552749021759982755929621040192561277219882014391768387;
    uint256 constant IC30y = 1678462193643989707904799880576318518424877650729617612886995466520645410295;

    uint256 constant IC31x = 12798577240683775281328059093526179322525098903189539874413899788121604685102;
    uint256 constant IC31y = 10657786561890994492801598140793796074968357570811172810854890586155824417490;

    uint256 constant IC32x = 9731619552788904952561421585119869131206116507127451756335593847921796649485;
    uint256 constant IC32y = 16040393627466945096690734713916640896860212625308282160922930663997304510160;

    uint256 constant IC33x = 13900349343058889539365003326987922130476007479778741609137524253774923525120;
    uint256 constant IC33y = 21110694516614242089417675173772199109546835051358208260535804554506534993403;

    uint256 constant IC34x = 16087168997759496863523038146191824722593885415559238316664816277239043343469;
    uint256 constant IC34y = 15035441443906381614443051662799319800610124331515297444790509178088366235938;

    uint256 constant IC35x = 18833031226160332231740160742750239314929090994531996328468596500488065451446;
    uint256 constant IC35y = 15486947418936973983832132527608302488871660111639775392892765185088914885795;

    uint256 constant IC36x = 9676221516555165443569993477435707765109306755218572920092713518443885586062;
    uint256 constant IC36y = 1337120519190588951112869757730195827478836537569628820635169664709018676585;

    uint256 constant IC37x = 5133728745403880071849075236052367444940412958080833230036120108604916769518;
    uint256 constant IC37y = 6815032371675477386325405873134591216138690390402193359435263281074277416019;

    uint256 constant IC38x = 15826318590016775865750868065762110923173551929869801115476303170451449691245;
    uint256 constant IC38y = 18559511032624865506462525411486546799805924567901549820577877395965537854251;

    uint256 constant IC39x = 6896941559762526269858400763859074574973818229459490651031641126783911478086;
    uint256 constant IC39y = 18198355501833723997722341140733825795619146562728010145438744124436286437278;

    uint256 constant IC40x = 16475935435173377577572192980190068281378971164883194843745508166261979397;
    uint256 constant IC40y = 7200904429112356399159177737086052410829678752162440041686459826987769012651;

    uint256 constant IC41x = 17603578280226371876808412477322837178212044112288698773027547692139430719780;
    uint256 constant IC41y = 4809267375298339938882813452150891611322230201837665606479236903022058869693;

    uint256 constant IC42x = 14063236221666791229694799676202196694540996133712266287306961728019880801313;
    uint256 constant IC42y = 10291356031911019878573819702575589099544233586300007146440608841180437344270;

    // Memory data
    uint16 constant pVk = 0;
    uint16 constant pPairing = 128;

    uint16 constant pLastMem = 896;

    function verifyProof(
        uint256[2] calldata _pA,
        uint256[2][2] calldata _pB,
        uint256[2] calldata _pC,
        uint256[42] calldata _pubSignals
    ) public view returns (bool) {
        assembly {
            function checkField(v) {
                if iszero(lt(v, r)) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            // G1 function to multiply a G1 value(x,y) to value in an address
            function g1_mulAccC(pR, x, y, s) {
                let success
                let mIn := mload(0x40)
                mstore(mIn, x)
                mstore(add(mIn, 32), y)
                mstore(add(mIn, 64), s)

                success := staticcall(sub(gas(), 2000), 7, mIn, 96, mIn, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }

                mstore(add(mIn, 64), mload(pR))
                mstore(add(mIn, 96), mload(add(pR, 32)))

                success := staticcall(sub(gas(), 2000), 6, mIn, 128, pR, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            function checkPairing(pA, pB, pC, pubSignals, pMem) -> isOk {
                let _pPairing := add(pMem, pPairing)
                let _pVk := add(pMem, pVk)

                mstore(_pVk, IC0x)
                mstore(add(_pVk, 32), IC0y)

                // Compute the linear combination vk_x

                g1_mulAccC(_pVk, IC1x, IC1y, calldataload(add(pubSignals, 0)))
                g1_mulAccC(_pVk, IC2x, IC2y, calldataload(add(pubSignals, 32)))
                g1_mulAccC(_pVk, IC3x, IC3y, calldataload(add(pubSignals, 64)))
                g1_mulAccC(_pVk, IC4x, IC4y, calldataload(add(pubSignals, 96)))
                g1_mulAccC(_pVk, IC5x, IC5y, calldataload(add(pubSignals, 128)))
                g1_mulAccC(_pVk, IC6x, IC6y, calldataload(add(pubSignals, 160)))
                g1_mulAccC(_pVk, IC7x, IC7y, calldataload(add(pubSignals, 192)))
                g1_mulAccC(_pVk, IC8x, IC8y, calldataload(add(pubSignals, 224)))
                g1_mulAccC(_pVk, IC9x, IC9y, calldataload(add(pubSignals, 256)))
                g1_mulAccC(_pVk, IC10x, IC10y, calldataload(add(pubSignals, 288)))
                g1_mulAccC(_pVk, IC11x, IC11y, calldataload(add(pubSignals, 320)))
                g1_mulAccC(_pVk, IC12x, IC12y, calldataload(add(pubSignals, 352)))
                g1_mulAccC(_pVk, IC13x, IC13y, calldataload(add(pubSignals, 384)))
                g1_mulAccC(_pVk, IC14x, IC14y, calldataload(add(pubSignals, 416)))
                g1_mulAccC(_pVk, IC15x, IC15y, calldataload(add(pubSignals, 448)))
                g1_mulAccC(_pVk, IC16x, IC16y, calldataload(add(pubSignals, 480)))
                g1_mulAccC(_pVk, IC17x, IC17y, calldataload(add(pubSignals, 512)))
                g1_mulAccC(_pVk, IC18x, IC18y, calldataload(add(pubSignals, 544)))
                g1_mulAccC(_pVk, IC19x, IC19y, calldataload(add(pubSignals, 576)))
                g1_mulAccC(_pVk, IC20x, IC20y, calldataload(add(pubSignals, 608)))
                g1_mulAccC(_pVk, IC21x, IC21y, calldataload(add(pubSignals, 640)))
                g1_mulAccC(_pVk, IC22x, IC22y, calldataload(add(pubSignals, 672)))
                g1_mulAccC(_pVk, IC23x, IC23y, calldataload(add(pubSignals, 704)))
                g1_mulAccC(_pVk, IC24x, IC24y, calldataload(add(pubSignals, 736)))
                g1_mulAccC(_pVk, IC25x, IC25y, calldataload(add(pubSignals, 768)))
                g1_mulAccC(_pVk, IC26x, IC26y, calldataload(add(pubSignals, 800)))
                g1_mulAccC(_pVk, IC27x, IC27y, calldataload(add(pubSignals, 832)))
                g1_mulAccC(_pVk, IC28x, IC28y, calldataload(add(pubSignals, 864)))
                g1_mulAccC(_pVk, IC29x, IC29y, calldataload(add(pubSignals, 896)))
                g1_mulAccC(_pVk, IC30x, IC30y, calldataload(add(pubSignals, 928)))
                g1_mulAccC(_pVk, IC31x, IC31y, calldataload(add(pubSignals, 960)))
                g1_mulAccC(_pVk, IC32x, IC32y, calldataload(add(pubSignals, 992)))
                g1_mulAccC(_pVk, IC33x, IC33y, calldataload(add(pubSignals, 1024)))
                g1_mulAccC(_pVk, IC34x, IC34y, calldataload(add(pubSignals, 1056)))
                g1_mulAccC(_pVk, IC35x, IC35y, calldataload(add(pubSignals, 1088)))
                g1_mulAccC(_pVk, IC36x, IC36y, calldataload(add(pubSignals, 1120)))
                g1_mulAccC(_pVk, IC37x, IC37y, calldataload(add(pubSignals, 1152)))
                g1_mulAccC(_pVk, IC38x, IC38y, calldataload(add(pubSignals, 1184)))
                g1_mulAccC(_pVk, IC39x, IC39y, calldataload(add(pubSignals, 1216)))
                g1_mulAccC(_pVk, IC40x, IC40y, calldataload(add(pubSignals, 1248)))
                g1_mulAccC(_pVk, IC41x, IC41y, calldataload(add(pubSignals, 1280)))
                g1_mulAccC(_pVk, IC42x, IC42y, calldataload(add(pubSignals, 1312)))

                // -A
                mstore(_pPairing, calldataload(pA))
                mstore(add(_pPairing, 32), mod(sub(q, calldataload(add(pA, 32))), q))

                // B
                mstore(add(_pPairing, 64), calldataload(pB))
                mstore(add(_pPairing, 96), calldataload(add(pB, 32)))
                mstore(add(_pPairing, 128), calldataload(add(pB, 64)))
                mstore(add(_pPairing, 160), calldataload(add(pB, 96)))

                // alpha1
                mstore(add(_pPairing, 192), alphax)
                mstore(add(_pPairing, 224), alphay)

                // beta2
                mstore(add(_pPairing, 256), betax1)
                mstore(add(_pPairing, 288), betax2)
                mstore(add(_pPairing, 320), betay1)
                mstore(add(_pPairing, 352), betay2)

                // vk_x
                mstore(add(_pPairing, 384), mload(add(pMem, pVk)))
                mstore(add(_pPairing, 416), mload(add(pMem, add(pVk, 32))))

                // gamma2
                mstore(add(_pPairing, 448), gammax1)
                mstore(add(_pPairing, 480), gammax2)
                mstore(add(_pPairing, 512), gammay1)
                mstore(add(_pPairing, 544), gammay2)

                // C
                mstore(add(_pPairing, 576), calldataload(pC))
                mstore(add(_pPairing, 608), calldataload(add(pC, 32)))

                // delta2
                mstore(add(_pPairing, 640), deltax1)
                mstore(add(_pPairing, 672), deltax2)
                mstore(add(_pPairing, 704), deltay1)
                mstore(add(_pPairing, 736), deltay2)

                let success := staticcall(sub(gas(), 2000), 8, _pPairing, 768, _pPairing, 0x20)

                isOk := and(success, mload(_pPairing))
            }

            let pMem := mload(0x40)
            mstore(0x40, add(pMem, pLastMem))

            // Validate that all evaluations ∈ F

            checkField(calldataload(add(_pubSignals, 0)))

            checkField(calldataload(add(_pubSignals, 32)))

            checkField(calldataload(add(_pubSignals, 64)))

            checkField(calldataload(add(_pubSignals, 96)))

            checkField(calldataload(add(_pubSignals, 128)))

            checkField(calldataload(add(_pubSignals, 160)))

            checkField(calldataload(add(_pubSignals, 192)))

            checkField(calldataload(add(_pubSignals, 224)))

            checkField(calldataload(add(_pubSignals, 256)))

            checkField(calldataload(add(_pubSignals, 288)))

            checkField(calldataload(add(_pubSignals, 320)))

            checkField(calldataload(add(_pubSignals, 352)))

            checkField(calldataload(add(_pubSignals, 384)))

            checkField(calldataload(add(_pubSignals, 416)))

            checkField(calldataload(add(_pubSignals, 448)))

            checkField(calldataload(add(_pubSignals, 480)))

            checkField(calldataload(add(_pubSignals, 512)))

            checkField(calldataload(add(_pubSignals, 544)))

            checkField(calldataload(add(_pubSignals, 576)))

            checkField(calldataload(add(_pubSignals, 608)))

            checkField(calldataload(add(_pubSignals, 640)))

            checkField(calldataload(add(_pubSignals, 672)))

            checkField(calldataload(add(_pubSignals, 704)))

            checkField(calldataload(add(_pubSignals, 736)))

            checkField(calldataload(add(_pubSignals, 768)))

            checkField(calldataload(add(_pubSignals, 800)))

            checkField(calldataload(add(_pubSignals, 832)))

            checkField(calldataload(add(_pubSignals, 864)))

            checkField(calldataload(add(_pubSignals, 896)))

            checkField(calldataload(add(_pubSignals, 928)))

            checkField(calldataload(add(_pubSignals, 960)))

            checkField(calldataload(add(_pubSignals, 992)))

            checkField(calldataload(add(_pubSignals, 1024)))

            checkField(calldataload(add(_pubSignals, 1056)))

            checkField(calldataload(add(_pubSignals, 1088)))

            checkField(calldataload(add(_pubSignals, 1120)))

            checkField(calldataload(add(_pubSignals, 1152)))

            checkField(calldataload(add(_pubSignals, 1184)))

            checkField(calldataload(add(_pubSignals, 1216)))

            checkField(calldataload(add(_pubSignals, 1248)))

            checkField(calldataload(add(_pubSignals, 1280)))

            checkField(calldataload(add(_pubSignals, 1312)))

            checkField(calldataload(add(_pubSignals, 1344)))

            // Validate all evaluations
            let isValid := checkPairing(_pA, _pB, _pC, _pubSignals, pMem)

            mstore(0, isValid)

            return(0, 0x20)
        }
    }
}

/* =============================== */
/* Nova+CycleFold Decider verifier */
/**
 * @notice  Computes the decomposition of a `uint256` into num_limbs limbs of bits_per_limb bits each.
 * @dev     Compatible with sonobe::folding-schemes::folding::circuits::nonnative::nonnative_field_to_field_elements.
 */
library LimbsDecomposition {
    function decompose(uint256 x) internal pure returns (uint256[5] memory) {
        uint256[5] memory limbs;
        for (uint8 i = 0; i < 5; i++) {
            limbs[i] = (x >> (55 * i)) & ((1 << 55) - 1);
        }
        return limbs;
    }
}

/**
 * @author PSE & 0xPARC
 * @title  Interface for the RootNovaDecider contract hiding proof details.
 * @dev    This interface enables calling the verifyNovaProof function without exposing the proof details.
 */
interface OpaqueDecider {
    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps, // number of folded steps (i)
        uint256[3] calldata initial_state, // initial IVC state (z0)
        uint256[3] calldata final_state, // IVC state after i steps (zi)
        uint256[25] calldata proof // the rest of the decider inputs
    ) external view returns (bool);

    /**
     * @notice  Verifies a Nova+CycleFold proof given all the proof inputs collected in a single array.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProof(uint256[32] calldata proof) external view returns (bool);
}

/**
 * @author  PSE & 0xPARC
 * @title   RootNovaDecider contract, for verifying Nova IVC SNARK proofs.
 * @dev     This is an askama template which, when templated, features a Groth16 and KZG10 verifiers from which this contract inherits.
 */
contract RootNovaDecider is Groth16Verifier, KZG10Verifier, OpaqueDecider {
    /**
     * @notice  Computes the linear combination of a and b with r as the coefficient.
     * @dev     All ops are done mod the BN254 scalar field prime
     */
    function rlc(uint256 a, uint256 r, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := addmod(a, mulmod(r, b, BN254_SCALAR_FIELD), BN254_SCALAR_FIELD)
        }
    }

    /**
     * @notice  Verifies a nova cyclefold proof consisting of two KZG proofs and of a groth16 proof.
     * @dev     The selector of this function is "dynamic", since it depends on `z_len`.
     */
    function verifyNovaProof(
        // inputs are grouped to prevent errors due stack too deep
        uint256[7] calldata i_z0_zi, // [i, z0, zi] where |z0| == |zi|
        uint256[4] calldata U_i_cmW_U_i_cmE, // [U_i_cmW[2], U_i_cmE[2]]
        uint256[2] calldata u_i_cmW, // [u_i_cmW[2]]
        uint256[3] calldata cmT_r, // [cmT[2], r]
        uint256[2] calldata pA, // groth16
        uint256[2][2] calldata pB, // groth16
        uint256[2] calldata pC, // groth16
        uint256[4] calldata challenge_W_challenge_E_kzg_evals, // [challenge_W, challenge_E, eval_W, eval_E]
        uint256[2][2] calldata kzg_proof // [proof_W, proof_E]
    ) public view returns (bool) {
        require(i_z0_zi[0] >= 2, "Folding: the number of folded steps should be at least 2");

        // from gamma_abc_len, we subtract 1.
        uint256[42] memory public_inputs;

        public_inputs[0] = 21824188546313360376267094377006447629000325834999510599629262248112637864112;
        public_inputs[1] = i_z0_zi[0];

        for (uint256 i = 0; i < 6; i++) {
            public_inputs[2 + i] = i_z0_zi[1 + i];
        }

        {
            // U_i.cmW + r * u_i.cmW
            uint256[2] memory mulScalarPoint = super.mulScalar([u_i_cmW[0], u_i_cmW[1]], cmT_r[2]);
            uint256[2] memory cmW = super.add([U_i_cmW_U_i_cmE[0], U_i_cmW_U_i_cmE[1]], mulScalarPoint);

            {
                uint256[5] memory cmW_x_limbs = LimbsDecomposition.decompose(cmW[0]);
                uint256[5] memory cmW_y_limbs = LimbsDecomposition.decompose(cmW[1]);

                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[8 + k] = cmW_x_limbs[k];
                    public_inputs[13 + k] = cmW_y_limbs[k];
                }
            }

            require(
                this.check(
                    cmW, kzg_proof[0], challenge_W_challenge_E_kzg_evals[0], challenge_W_challenge_E_kzg_evals[2]
                ),
                "KZG: verifying proof for challenge W failed"
            );
        }

        {
            // U_i.cmE + r * cmT
            uint256[2] memory mulScalarPoint = super.mulScalar([cmT_r[0], cmT_r[1]], cmT_r[2]);
            uint256[2] memory cmE = super.add([U_i_cmW_U_i_cmE[2], U_i_cmW_U_i_cmE[3]], mulScalarPoint);

            {
                uint256[5] memory cmE_x_limbs = LimbsDecomposition.decompose(cmE[0]);
                uint256[5] memory cmE_y_limbs = LimbsDecomposition.decompose(cmE[1]);

                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[18 + k] = cmE_x_limbs[k];
                    public_inputs[23 + k] = cmE_y_limbs[k];
                }
            }

            require(
                this.check(
                    cmE, kzg_proof[1], challenge_W_challenge_E_kzg_evals[1], challenge_W_challenge_E_kzg_evals[3]
                ),
                "KZG: verifying proof for challenge E failed"
            );
        }

        {
            // add challenges
            public_inputs[28] = challenge_W_challenge_E_kzg_evals[0];
            public_inputs[29] = challenge_W_challenge_E_kzg_evals[1];
            public_inputs[30] = challenge_W_challenge_E_kzg_evals[2];
            public_inputs[31] = challenge_W_challenge_E_kzg_evals[3];

            uint256[5] memory cmT_x_limbs;
            uint256[5] memory cmT_y_limbs;

            cmT_x_limbs = LimbsDecomposition.decompose(cmT_r[0]);
            cmT_y_limbs = LimbsDecomposition.decompose(cmT_r[1]);

            for (uint8 k = 0; k < 5; k++) {
                public_inputs[28 + 4 + k] = cmT_x_limbs[k];
                public_inputs[33 + 4 + k] = cmT_y_limbs[k];
            }

            bool success_g16 = this.verifyProof(pA, pB, pC, public_inputs);
            require(success_g16 == true, "Groth16: verifying proof failed");
        }

        return (true);
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps,
        uint256[3] calldata initial_state,
        uint256[3] calldata final_state,
        uint256[25] calldata proof
    ) public view override returns (bool) {
        uint256[1 + 2 * 3] memory i_z0_zi;
        i_z0_zi[0] = steps;
        for (uint256 i = 0; i < 3; i++) {
            i_z0_zi[i + 1] = initial_state[i];
            i_z0_zi[i + 1 + 3] = final_state[i];
        }

        uint256[4] memory U_i_cmW_U_i_cmE = [proof[0], proof[1], proof[2], proof[3]];
        uint256[2] memory u_i_cmW = [proof[4], proof[5]];
        uint256[3] memory cmT_r = [proof[6], proof[7], proof[8]];
        uint256[2] memory pA = [proof[9], proof[10]];
        uint256[2][2] memory pB = [[proof[11], proof[12]], [proof[13], proof[14]]];
        uint256[2] memory pC = [proof[15], proof[16]];
        uint256[4] memory challenge_W_challenge_E_kzg_evals = [proof[17], proof[18], proof[19], proof[20]];
        uint256[2][2] memory kzg_proof = [[proof[21], proof[22]], [proof[23], proof[24]]];

        return this.verifyNovaProof(
            i_z0_zi, U_i_cmW_U_i_cmE, u_i_cmW, cmT_r, pA, pB, pC, challenge_W_challenge_E_kzg_evals, kzg_proof
        );
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given all proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProof(uint256[32] calldata proof) public view override returns (bool) {
        uint256[3] memory z0;
        uint256[3] memory zi;
        for (uint256 i = 0; i < 3; i++) {
            z0[i] = proof[i + 1];
            zi[i] = proof[i + 1 + 3];
        }

        uint256[25] memory extracted_proof;
        for (uint256 i = 0; i < 25; i++) {
            extracted_proof[i] = proof[7 + i];
        }

        return this.verifyOpaqueNovaProofWithInputs(proof[0], z0, zi, extracted_proof);
    }
}
