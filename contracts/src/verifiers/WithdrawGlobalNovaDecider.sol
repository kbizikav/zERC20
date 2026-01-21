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
    Additionally we implement the WithdrawGlobalNovaDecider contract, which combines the
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

    uint256 constant IC0x = 15052307815747625810718504059180609338276268496400294490741246920915115358570;
    uint256 constant IC0y = 18493934659650193691177273722168665803445208493747908540879068204556606799914;

    uint256 constant IC1x = 15143097026700358751832624319745148794048128457425613816563097127577962412810;
    uint256 constant IC1y = 20732284937489830006583804122164019600548511232860784435764829058995685913871;

    uint256 constant IC2x = 1560925308254298360827264858011769357394810139959634481239255997962791762933;
    uint256 constant IC2y = 18756963010565470158321905453434122418406035333634448939495543449674464970641;

    uint256 constant IC3x = 9310407341176653238780634995805507053628835689033626792421735118513771321799;
    uint256 constant IC3y = 2828565775715544085254942399065286029196509629996661109048052841262687790541;

    uint256 constant IC4x = 19143845722960438826224028869684683692400916971248687910296034943791979511434;
    uint256 constant IC4y = 17315178426171610502855568359384453801540996898765766566809767181562128017940;

    uint256 constant IC5x = 19855986888358713756313162143323764751907186719837319141817585976121949734344;
    uint256 constant IC5y = 20652870976136033877223939058912062511653577084853052088561677409687743030406;

    uint256 constant IC6x = 345504288700633694770860886187345095073259899446910342426190745925572464569;
    uint256 constant IC6y = 2144531080606212932980567959084706185473943401428860792972009295760435115400;

    uint256 constant IC7x = 17073408864026131994906928752639824450731877109370402054540707165056456626590;
    uint256 constant IC7y = 5035331716469876564307085828531734054711131612930621506178583204666291515961;

    uint256 constant IC8x = 4904189195289214400743967860643009316402213476427432912438397609567814172179;
    uint256 constant IC8y = 17535330019729960388619023013017260331217839268018358917028436592942631465155;

    uint256 constant IC9x = 21548375809064495160061078000108962295605053118963430825603650291097179278319;
    uint256 constant IC9y = 3208859432594786069090511865476200966867395416784083545226819462287466895832;

    uint256 constant IC10x = 14826883009368058704671547860468385935153338338048986942839938260771136439369;
    uint256 constant IC10y = 12891659958881967982143781835384634927118566893727576473495479041514004900695;

    uint256 constant IC11x = 17117880902400849850037714571924469697014719152782417944616542404171851961097;
    uint256 constant IC11y = 8380754963508551128431969211668871498909486036021696786715116220067598066484;

    uint256 constant IC12x = 6280125017346486975618249964448822674591269044134000935869594235008002505728;
    uint256 constant IC12y = 3961317420147157578334564973001563793011992207324756973730055215095426563732;

    uint256 constant IC13x = 10726857286743536320522744537001407603002541495610470216665006808962625970839;
    uint256 constant IC13y = 6663081233808820648425443831789745814226823451039547465779993980906187070541;

    uint256 constant IC14x = 19435267807836161217854683799291209852787471326406376999216477928365653291236;
    uint256 constant IC14y = 4317098003098015734510283843427914977454001392300025328196023700894733671154;

    uint256 constant IC15x = 11350290449123445024283473638599593046263289869684103697506103629613043511704;
    uint256 constant IC15y = 18886559103491902659725563167807318323441939344565230513230634052123454041470;

    uint256 constant IC16x = 7658171214888701031218677174392480372208430003913491935099919533356037745414;
    uint256 constant IC16y = 20125454215218871654813645422414423708050585433672992526288581491653650295267;

    uint256 constant IC17x = 20205025746686258506211876912968729687162093276785364621130169758807949008776;
    uint256 constant IC17y = 3743887041241439555365149855753471154639869631773184353929959868864602376135;

    uint256 constant IC18x = 10721501608522453462902466044617887487623015110359845515484452569883480262602;
    uint256 constant IC18y = 18515462695119471654227410677078446310436794919882120939978551331806334037258;

    uint256 constant IC19x = 11104847009992942264548545176025894647039763245341931843077225731461126504672;
    uint256 constant IC19y = 9179099639067884896826768797344608493618909384739444559402969792125175952932;

    uint256 constant IC20x = 11875234768743861953935973617987729072741935272713562907896201496558273722060;
    uint256 constant IC20y = 4994682621290915301513908816571561617324980783780317026151864360901190396319;

    uint256 constant IC21x = 3816362699367385932889889763558380391161233483335431804337610072517825383811;
    uint256 constant IC21y = 16727698288083562607334645466838527514468153886344148377982917212953469517917;

    uint256 constant IC22x = 2862766483143172301700773927484953710177009146119915890817743778154403871348;
    uint256 constant IC22y = 12651584481136961132102796249204435232111920320646145212116359421404689403706;

    uint256 constant IC23x = 19222867171273427306367343103905350082487524755105819423943253966437508497839;
    uint256 constant IC23y = 16035209781054067359224567732841756063984047884056970126056006409251432915684;

    uint256 constant IC24x = 20587356664525211332993242652470845750770472106173386394392442734554594160923;
    uint256 constant IC24y = 14908427071783722176153555288218227263379051916055663244921900773490341667900;

    uint256 constant IC25x = 697795743567540366110110576506026089895630872116351867354096661501208402251;
    uint256 constant IC25y = 15113554977994754417151578855496664463882150443410365408558511871097707196024;

    uint256 constant IC26x = 5805219415849933907899324899172220340538912075963724984598305870198075300466;
    uint256 constant IC26y = 9148662068804650693473074431480799095041123320876495986695569555758050278742;

    uint256 constant IC27x = 8014087637654155995338613283813077897223307610909854008040715703127556472493;
    uint256 constant IC27y = 2920665841535583753836567798769619660873668592473396535583967879617006766325;

    uint256 constant IC28x = 11003773629631972338640097792742461275389364642851846136613413970972497594558;
    uint256 constant IC28y = 21390930304671334739580889996011767272483397956409806827179903025141741891498;

    uint256 constant IC29x = 10314277924605609424698011600243211375856947445685773790656777422539541778963;
    uint256 constant IC29y = 14944153744967553685960493202107709170535790800596614832588980843405125207531;

    uint256 constant IC30x = 12203907945518608273092070603524927507100371071927043564259513149688560878152;
    uint256 constant IC30y = 15428982064706148182647385388382574394561330904937938254208529955128217677150;

    uint256 constant IC31x = 18616139129655631881627251237799344649443828606867805557890683205183989773787;
    uint256 constant IC31y = 5590925813146762814768373420599778331850724147222900954205414189058673173237;

    uint256 constant IC32x = 14894303355875004134838821552090968861382077226391582129102921980384830399002;
    uint256 constant IC32y = 21519181597210239558168764059409877432264920226768149978907236686172148417560;

    uint256 constant IC33x = 13546465926089239478982902953642354508737939489957914401976119175346066613133;
    uint256 constant IC33y = 5598268759616562942949893297438242349957882867734167661678553496538085504726;

    uint256 constant IC34x = 17889160305353578180132845539907344133503965473321813279748398317563979691322;
    uint256 constant IC34y = 13034697130641431442441805300693518363879487470999956245041186871544912822239;

    uint256 constant IC35x = 8496738472840448862528174304267102091084762313654684074054676253007340119514;
    uint256 constant IC35y = 5456671011002389572953142784316325461036418281231908256424104724017444999434;

    uint256 constant IC36x = 17037133082358911438324801882645852313586712310674888744859521819653588369895;
    uint256 constant IC36y = 14095276424020871911047124551443850412496244554581902583603030905942064111914;

    uint256 constant IC37x = 20148639978726049280656550500063083172096205985877780011491160659855639654854;
    uint256 constant IC37y = 4290505103344526391622391764835934578023866296345483694220968260178798915339;

    uint256 constant IC38x = 4714237497208485496537898477579020290056086790716399554476332048257045643524;
    uint256 constant IC38y = 9385554659875102409525352304244081191108420773440657039867312779906812839731;

    uint256 constant IC39x = 6323304937394613299064427119878574042179585140798284878099800157510690368853;
    uint256 constant IC39y = 3823071602189553427514181076629447015936426837905575822510840332291397925957;

    uint256 constant IC40x = 16880055334113384243238650490416368422354212983633776697356973147973858145265;
    uint256 constant IC40y = 10406393433903552705018032044828805569472130751998878754944873542486748864267;

    uint256 constant IC41x = 9590298163903286398513916291977214585850107823518055907791397059587375718929;
    uint256 constant IC41y = 2825893605911495720213494925791133434632152113627472084767395346177544810832;

    uint256 constant IC42x = 15133528944431060109584159539493504526439842727494284903375166449987552253722;
    uint256 constant IC42y = 16257174013244821487760419943333796354377613032211675819070221441024695513879;

    uint256 constant IC43x = 12952359772208266286176917086369362671451804093979627969500060887652611970582;
    uint256 constant IC43y = 21229303716195206299746200442546005485806822241108216162838080968939514208511;

    uint256 constant IC44x = 19351706682621457038086935550557719347525743033793751631504094379029744217414;
    uint256 constant IC44y = 4587324783301899594015453639115614644007529179871590443535398476065413409436;

    // Memory data
    uint16 constant pVk = 0;
    uint16 constant pPairing = 128;

    uint16 constant pLastMem = 896;

    function verifyProof(
        uint256[2] calldata _pA,
        uint256[2][2] calldata _pB,
        uint256[2] calldata _pC,
        uint256[44] calldata _pubSignals
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
                g1_mulAccC(_pVk, IC43x, IC43y, calldataload(add(pubSignals, 1344)))
                g1_mulAccC(_pVk, IC44x, IC44y, calldataload(add(pubSignals, 1376)))

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

            checkField(calldataload(add(_pubSignals, 1376)))

            checkField(calldataload(add(_pubSignals, 1408)))

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
 * @title  Interface for the WithdrawGlobalNovaDecider contract hiding proof details.
 * @dev    This interface enables calling the verifyNovaProof function without exposing the proof details.
 */
interface OpaqueDecider {
    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps, // number of folded steps (i)
        uint256[4] calldata initial_state, // initial IVC state (z0)
        uint256[4] calldata final_state, // IVC state after i steps (zi)
        uint256[25] calldata proof // the rest of the decider inputs
    ) external view returns (bool);

    /**
     * @notice  Verifies a Nova+CycleFold proof given all the proof inputs collected in a single array.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProof(uint256[34] calldata proof) external view returns (bool);
}

/**
 * @author  PSE & 0xPARC
 * @title   WithdrawGlobalNovaDecider contract, for verifying Nova IVC SNARK proofs.
 * @dev     This is an askama template which, when templated, features a Groth16 and KZG10 verifiers from which this contract inherits.
 */
contract WithdrawGlobalNovaDecider is Groth16Verifier, KZG10Verifier, OpaqueDecider {
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
        uint256[9] calldata i_z0_zi, // [i, z0, zi] where |z0| == |zi|
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
        uint256[44] memory public_inputs;

        public_inputs[0] = 6695724418339755650712062445012036119377640839478600263530571696147014176926;
        public_inputs[1] = i_z0_zi[0];

        for (uint256 i = 0; i < 8; i++) {
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
                    public_inputs[10 + k] = cmW_x_limbs[k];
                    public_inputs[15 + k] = cmW_y_limbs[k];
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
                    public_inputs[20 + k] = cmE_x_limbs[k];
                    public_inputs[25 + k] = cmE_y_limbs[k];
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
            public_inputs[30] = challenge_W_challenge_E_kzg_evals[0];
            public_inputs[31] = challenge_W_challenge_E_kzg_evals[1];
            public_inputs[32] = challenge_W_challenge_E_kzg_evals[2];
            public_inputs[33] = challenge_W_challenge_E_kzg_evals[3];

            uint256[5] memory cmT_x_limbs;
            uint256[5] memory cmT_y_limbs;

            cmT_x_limbs = LimbsDecomposition.decompose(cmT_r[0]);
            cmT_y_limbs = LimbsDecomposition.decompose(cmT_r[1]);

            for (uint8 k = 0; k < 5; k++) {
                public_inputs[30 + 4 + k] = cmT_x_limbs[k];
                public_inputs[35 + 4 + k] = cmT_y_limbs[k];
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
        uint256[4] calldata initial_state,
        uint256[4] calldata final_state,
        uint256[25] calldata proof
    ) public view override returns (bool) {
        uint256[1 + 2 * 4] memory i_z0_zi;
        i_z0_zi[0] = steps;
        for (uint256 i = 0; i < 4; i++) {
            i_z0_zi[i + 1] = initial_state[i];
            i_z0_zi[i + 1 + 4] = final_state[i];
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
    function verifyOpaqueNovaProof(uint256[34] calldata proof) public view override returns (bool) {
        uint256[4] memory z0;
        uint256[4] memory zi;
        for (uint256 i = 0; i < 4; i++) {
            z0[i] = proof[i + 1];
            zi[i] = proof[i + 1 + 4];
        }

        uint256[25] memory extracted_proof;
        for (uint256 i = 0; i < 25; i++) {
            extracted_proof[i] = proof[9 + i];
        }

        return this.verifyOpaqueNovaProofWithInputs(proof[0], z0, zi, extracted_proof);
    }
}
